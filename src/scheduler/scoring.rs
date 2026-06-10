use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq)]
pub struct SchedulerScoreWeights {
    pub priority: f64,
    pub load: f64,
    pub queue: f64,
    pub error_rate: f64,
    pub latency: f64,
    pub health: f64,
}

impl Default for SchedulerScoreWeights {
    fn default() -> Self {
        Self {
            priority: 1.0,
            load: 1.2,
            queue: 0.8,
            error_rate: 1.5,
            latency: 0.6,
            health: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CandidateScoreInput {
    pub candidate_id: String,
    pub priority: i32,
    pub load_rate: u32,
    pub waiting_count: u32,
    pub max_waiting_count: u32,
    pub error_rate: f64,
    pub latency_ms: Option<f64>,
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
    // Priority is normalized by a simple inverse curve so lower priority values score higher
    // without requiring the whole candidate set.
    let priority_factor = 1.0 / (1.0 + (input.priority.max(0) as f64));
    let load_factor = 1.0 - clamp01(input.load_rate as f64 / 100.0);
    let max_waiting = input.max_waiting_count.max(1) as f64;
    let queue_factor = 1.0 - clamp01(input.waiting_count as f64 / max_waiting);
    let error_factor = 1.0 - clamp01(input.error_rate);
    let latency_factor = match (input.latency_ms, input.min_latency_ms, input.max_latency_ms) {
        (Some(latency), Some(min), Some(max)) if max > min => {
            1.0 - clamp01((latency - min) / (max - min))
        }
        (Some(_), _, _) => 1.0,
        _ => 0.5,
    };
    let health_factor = clamp01(input.health);

    weights.priority * priority_factor
        + weights.load * load_factor
        + weights.queue * queue_factor
        + weights.error_rate * error_factor
        + weights.latency * latency_factor
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
        .then_with(|| a.input.priority.cmp(&b.input.priority))
        .then_with(|| a.input.load_rate.cmp(&b.input.load_rate))
        .then_with(|| a.input.waiting_count.cmp(&b.input.waiting_count))
        .then_with(|| a.input.candidate_id.cmp(&b.input.candidate_id))
}

