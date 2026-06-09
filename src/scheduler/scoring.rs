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

#[derive(Debug, Clone)]
struct SelectionRng {
    state: u64,
}

impl SelectionRng {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0x9e37_79b9_7f4a_7c15
            } else {
                seed
            },
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(2_685_821_657_736_338_717)
    }

    fn next_f64(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64) / ((1u64 << 53) as f64)
    }
}

/// Builds a weighted attempt order from already-ranked top-K candidates.
///
/// The returned vector contains every input candidate exactly once. Scores are shifted
/// into a positive range so lower-scored top-K candidates still have a chance, avoiding
/// permanent monopoly by the best candidate while keeping better candidates more likely
/// to appear earlier.
pub fn build_weighted_selection_order(
    candidates: &[ScoredCandidate],
    seed: u64,
) -> Vec<ScoredCandidate> {
    if candidates.len() <= 1 {
        return candidates.to_vec();
    }

    let mut pool = candidates.to_vec();
    let min_score = pool.iter().map(|c| c.score).fold(f64::INFINITY, f64::min);
    let mut weights: Vec<f64> = pool
        .iter()
        .map(|c| {
            let weight = (c.score - min_score) + 1.0;
            if weight.is_finite() && weight > 0.0 {
                weight
            } else {
                1.0
            }
        })
        .collect();
    let mut rng = SelectionRng::new(seed);
    let mut order = Vec::with_capacity(pool.len());

    while !pool.is_empty() {
        let total: f64 = weights.iter().sum();
        let selected_idx = if total > 0.0 && total.is_finite() {
            let mut threshold = rng.next_f64() * total;
            let mut idx = 0;
            for (i, weight) in weights.iter().enumerate() {
                threshold -= *weight;
                if threshold <= 0.0 {
                    idx = i;
                    break;
                }
            }
            idx
        } else {
            (rng.next_u64() as usize) % pool.len()
        };

        order.push(pool.remove(selected_idx));
        weights.remove(selected_idx);
    }

    order
}
