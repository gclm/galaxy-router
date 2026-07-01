use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq)]
pub struct SchedulerScoreWeights {
    pub latency: f64,
    pub least_conn: f64,
    pub error_rate: f64,
    pub health: f64,
}

impl Default for SchedulerScoreWeights {
    fn default() -> Self {
        // latency 与 least_conn 等权（速度优先 + 并发分散两大核心诉求）；
        // error_rate/health 略高（可用性底线）。起点参数，上线后按 metrics 调。
        Self {
            latency: 1.0,
            least_conn: 1.0,
            error_rate: 1.5,
            health: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CandidateScoreInput {
    pub candidate_id: String,
    /// 实时在途并发数（最小连接数打分）
    pub active_concurrency: u32,
    /// 候选集内最小/最大在途并发（相对归一化用，由 candidates.rs 两遍扫填入）
    pub min_active: Option<f64>,
    pub max_active: Option<f64>,
    pub error_rate: f64,
    pub latency_ms: Option<f64>,
    /// 候选集内最小/最大延迟（相对归一化用，由 candidates.rs 两遍扫填入）
    pub min_latency_ms: Option<f64>,
    pub max_latency_ms: Option<f64>,
    /// 0.0 = unhealthy, 1.0 = healthy.
    pub health: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScoredCandidate {
    pub input: CandidateScoreInput,
    pub score: f64,
}

fn clamp01(value: f64) -> f64 {
    if value.is_nan() {
        return 0.0;
    }
    value.clamp(0.0, 1.0)
}

pub fn calculate_candidate_score(
    input: &CandidateScoreInput,
    weights: &SchedulerScoreWeights,
) -> f64 {
    // latency：候选集内相对归一化。无数据冷启动 0.8（不埋没新渠道，也不反超已知最快）。
    let latency_factor = match (input.latency_ms, input.min_latency_ms, input.max_latency_ms) {
        (Some(latency), Some(min), Some(max)) if max > min => {
            1.0 - clamp01((latency - min) / (max - min))
        }
        (Some(_), _, _) => 1.0, // 有数据但集内无可比（单候选），不惩罚
        _ => 0.8,               // 无数据冷启动
    };
    // 最小连接数：候选集内相对归一化，active 少打分高。全 0 或单候选不惩罚。
    let least_conn_factor = match (input.min_active, input.max_active) {
        (Some(min), Some(max)) if max > min => {
            1.0 - clamp01((input.active_concurrency as f64 - min) / (max - min))
        }
        _ => 1.0,
    };
    let error_factor = 1.0 - clamp01(input.error_rate);
    let health_factor = clamp01(input.health);

    weights.latency * latency_factor
        + weights.least_conn * least_conn_factor
        + weights.error_rate * error_factor
        + weights.health * health_factor
}

pub fn select_top_k_candidates(
    candidates: &[CandidateScoreInput],
    top_k: usize,
    weights: &SchedulerScoreWeights,
) -> Vec<ScoredCandidate> {
    if candidates.is_empty() || top_k == 0 {
        return Vec::new();
    }

    let mut scored: Vec<ScoredCandidate> = candidates
        .iter()
        .cloned()
        .map(|input| {
            let score = calculate_candidate_score(&input, weights);
            ScoredCandidate { input, score }
        })
        .collect();

    scored.sort_by(compare_scored_candidates);
    scored.truncate(top_k.min(scored.len()));
    scored
}

fn compare_scored_candidates(a: &ScoredCandidate, b: &ScoredCandidate) -> Ordering {
    b.score
        .partial_cmp(&a.score)
        .unwrap_or(Ordering::Equal)
        .then_with(|| a.input.candidate_id.cmp(&b.input.candidate_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn input(
        id: &str,
        active: u32,
        min_a: Option<f64>,
        max_a: Option<f64>,
        lat: Option<f64>,
        min_l: Option<f64>,
        max_l: Option<f64>,
        err: f64,
        health: f64,
    ) -> CandidateScoreInput {
        CandidateScoreInput {
            candidate_id: id.into(),
            active_concurrency: active,
            min_active: min_a,
            max_active: max_a,
            error_rate: err,
            latency_ms: lat,
            min_latency_ms: min_l,
            max_latency_ms: max_l,
            health,
        }
    }

    /// latency 候选集内相对归一化：低延迟打分高
    #[test]
    fn latency_relative_normalization_prefers_fast() {
        let w = SchedulerScoreWeights::default();
        let fast = input(
            "fast", 0, Some(0.0), Some(0.0), Some(100.0), Some(100.0), Some(1000.0), 0.0, 1.0,
        );
        let slow = input(
            "slow", 0, Some(0.0), Some(0.0), Some(1000.0), Some(100.0), Some(1000.0), 0.0, 1.0,
        );
        assert!(calculate_candidate_score(&fast, &w) > calculate_candidate_score(&slow, &w));
    }

    /// latency 冷启动：无数据给 0.8，不被 0.5 埋没
    #[test]
    fn latency_cold_start_not_buried() {
        let w = SchedulerScoreWeights::default();
        let cold = input("cold", 0, None, None, None, None, None, 0.0, 1.0);
        // 0.8*1.0(latency) + 1.0*1.0(least_conn) + 1.5*1.0(error) + 1.0*1.0(health) = 4.3
        let score = calculate_candidate_score(&cold, &w);
        assert!(
            score > 4.0,
            "cold-start latency_factor=0.8 should not bury candidate, got {score}"
        );
    }

    /// 最小连接数：active 少打分高
    #[test]
    fn least_conn_prefers_idle() {
        let w = SchedulerScoreWeights::default();
        let idle = input(
            "idle", 0, Some(0.0), Some(5.0), Some(100.0), Some(100.0), Some(100.0), 0.0, 1.0,
        );
        let busy = input(
            "busy", 5, Some(0.0), Some(5.0), Some(100.0), Some(100.0), Some(100.0), 0.0, 1.0,
        );
        assert!(calculate_candidate_score(&idle, &w) > calculate_candidate_score(&busy, &w));
    }

    /// 最小连接数：全 0 active 不惩罚（least_conn_factor=1.0）
    #[test]
    fn least_conn_all_zero_not_punished() {
        let w = SchedulerScoreWeights::default();
        let a = input("a", 0, Some(0.0), Some(0.0), None, None, None, 0.0, 1.0);
        let b = input("b", 0, Some(0.0), Some(0.0), None, None, None, 0.0, 1.0);
        assert_eq!(calculate_candidate_score(&a, &w), calculate_candidate_score(&b, &w));
    }

    /// health=0 打分低于 health=1
    #[test]
    fn health_zero_lower_score() {
        let w = SchedulerScoreWeights::default();
        let healthy = input("h", 0, None, None, None, None, None, 0.0, 1.0);
        let unhealthy = input("u", 0, None, None, None, None, None, 0.0, 0.0);
        assert!(calculate_candidate_score(&healthy, &w) > calculate_candidate_score(&unhealthy, &w));
    }
}

