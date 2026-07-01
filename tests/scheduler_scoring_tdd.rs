use galaxy_router::scheduler::scoring::{
    CandidateScoreInput, SchedulerScoreWeights, calculate_candidate_score, select_top_k_candidates,
};

fn weights() -> SchedulerScoreWeights {
    SchedulerScoreWeights {
        latency: 1.0,
        least_conn: 1.0,
        error_rate: 1.0,
        health: 1.0,
    }
}

#[allow(clippy::too_many_arguments)]
fn input(
    id: &str,
    active: u32,
    min_a: f64,
    max_a: f64,
    lat: f64,
    min_l: f64,
    max_l: f64,
    err: f64,
    health: f64,
) -> CandidateScoreInput {
    CandidateScoreInput {
        candidate_id: id.into(),
        active_concurrency: active,
        min_active: Some(min_a),
        max_active: Some(max_a),
        error_rate: err,
        latency_ms: Some(lat),
        min_latency_ms: Some(min_l),
        max_latency_ms: Some(max_l),
        health,
    }
}

/// 最小连接数：active 少的（least_conn_factor 高）打分高
#[test]
fn scheduler_scoring_prefers_lower_concurrency_when_other_factors_equal() {
    let idle = input("idle", 0, 0.0, 10.0, 100.0, 100.0, 100.0, 0.0, 1.0);
    let busy = input("busy", 10, 0.0, 10.0, 100.0, 100.0, 100.0, 0.0, 1.0);
    assert!(
        calculate_candidate_score(&idle, &weights())
            > calculate_candidate_score(&busy, &weights())
    );
}

/// error_rate：低错误率打分高
#[test]
fn scheduler_scoring_prefers_lower_error_rate_when_other_factors_equal() {
    let healthy = input("healthy", 0, 0.0, 0.0, 100.0, 100.0, 100.0, 0.0, 1.0);
    let failing = input("failing", 0, 0.0, 0.0, 100.0, 100.0, 100.0, 0.8, 1.0);
    assert!(
        calculate_candidate_score(&healthy, &weights())
            > calculate_candidate_score(&failing, &weights())
    );
}

/// top_k：按 score 降序取前 k（此处由 latency 相对归一化驱动）
#[test]
fn scheduler_scoring_top_k_orders_by_score() {
    // 集内 latency min=100, max=1000：best(100)=1.0, middle(500)=~0.56, slow(1000)=0
    let candidates = vec![
        input("slow", 0, 0.0, 0.0, 1000.0, 100.0, 1000.0, 0.0, 1.0),
        input("best", 0, 0.0, 0.0, 100.0, 100.0, 1000.0, 0.0, 1.0),
        input("middle", 0, 0.0, 0.0, 500.0, 100.0, 1000.0, 0.0, 1.0),
    ];
    let selected = select_top_k_candidates(&candidates, 2, &weights());
    let ids: Vec<_> = selected
        .iter()
        .map(|c| c.input.candidate_id.as_str())
        .collect();
    assert_eq!(ids, vec!["best", "middle"]);
}
