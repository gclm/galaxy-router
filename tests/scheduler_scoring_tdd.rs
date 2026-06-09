use galaxy_router::scheduler::scoring::{
    CandidateScoreInput, SchedulerScoreWeights, calculate_candidate_score, select_top_k_candidates,
};

fn weights() -> SchedulerScoreWeights {
    SchedulerScoreWeights {
        priority: 1.0,
        load: 1.0,
        queue: 1.0,
        error_rate: 1.0,
        latency: 1.0,
        health: 1.0,
    }
}

#[test]
fn scheduler_scoring_prefers_lower_load_when_other_factors_equal() {
    let low_load = CandidateScoreInput {
        candidate_id: "low".into(),
        priority: 1,
        load_rate: 10,
        waiting_count: 0,
        max_waiting_count: 1,
        error_rate: 0.0,
        latency_ms: Some(100.0),
        min_latency_ms: Some(100.0),
        max_latency_ms: Some(100.0),
        health: 1.0,
    };
    let high_load = CandidateScoreInput {
        load_rate: 90,
        candidate_id: "high".into(),
        ..low_load.clone()
    };

    assert!(
        calculate_candidate_score(&low_load, &weights())
            > calculate_candidate_score(&high_load, &weights())
    );
}

#[test]
fn scheduler_scoring_prefers_lower_error_rate_when_other_factors_equal() {
    let healthy = CandidateScoreInput {
        candidate_id: "healthy".into(),
        priority: 1,
        load_rate: 20,
        waiting_count: 0,
        max_waiting_count: 1,
        error_rate: 0.0,
        latency_ms: Some(100.0),
        min_latency_ms: Some(100.0),
        max_latency_ms: Some(100.0),
        health: 1.0,
    };
    let failing = CandidateScoreInput {
        error_rate: 0.8,
        candidate_id: "failing".into(),
        ..healthy.clone()
    };

    assert!(
        calculate_candidate_score(&healthy, &weights())
            > calculate_candidate_score(&failing, &weights())
    );
}

#[test]
fn scheduler_scoring_top_k_orders_by_score_then_stable_tiebreakers() {
    let base = CandidateScoreInput {
        candidate_id: "base".into(),
        priority: 2,
        load_rate: 50,
        waiting_count: 0,
        max_waiting_count: 1,
        error_rate: 0.0,
        latency_ms: Some(100.0),
        min_latency_ms: Some(100.0),
        max_latency_ms: Some(100.0),
        health: 1.0,
    };
    let candidates = vec![
        CandidateScoreInput {
            candidate_id: "slow".into(),
            load_rate: 90,
            ..base.clone()
        },
        CandidateScoreInput {
            candidate_id: "best".into(),
            priority: 1,
            load_rate: 10,
            ..base.clone()
        },
        CandidateScoreInput {
            candidate_id: "middle".into(),
            priority: 1,
            load_rate: 40,
            ..base.clone()
        },
    ];

    let selected = select_top_k_candidates(&candidates, 2, &weights());
    let ids: Vec<_> = selected
        .iter()
        .map(|c| c.input.candidate_id.as_str())
        .collect();

    assert_eq!(ids, vec!["best", "middle"]);
}

use galaxy_router::scheduler::scoring::build_weighted_selection_order;

#[test]
fn scheduler_weighted_order_includes_each_top_k_candidate_once() {
    let base = CandidateScoreInput {
        candidate_id: "base".into(),
        priority: 1,
        load_rate: 10,
        waiting_count: 0,
        max_waiting_count: 1,
        error_rate: 0.0,
        latency_ms: Some(100.0),
        min_latency_ms: Some(100.0),
        max_latency_ms: Some(100.0),
        health: 1.0,
    };
    let candidates = vec![
        CandidateScoreInput {
            candidate_id: "a".into(),
            load_rate: 10,
            ..base.clone()
        },
        CandidateScoreInput {
            candidate_id: "b".into(),
            load_rate: 20,
            ..base.clone()
        },
        CandidateScoreInput {
            candidate_id: "c".into(),
            load_rate: 30,
            ..base.clone()
        },
    ];
    let top = select_top_k_candidates(&candidates, 3, &weights());

    let order = build_weighted_selection_order(&top, 42);
    let mut ids: Vec<_> = order
        .iter()
        .map(|c| c.input.candidate_id.as_str())
        .collect();
    ids.sort_unstable();

    assert_eq!(ids, vec!["a", "b", "c"]);
}

#[test]
fn scheduler_weighted_order_is_reproducible_for_same_seed() {
    let base = CandidateScoreInput {
        candidate_id: "base".into(),
        priority: 1,
        load_rate: 10,
        waiting_count: 0,
        max_waiting_count: 1,
        error_rate: 0.0,
        latency_ms: Some(100.0),
        min_latency_ms: Some(100.0),
        max_latency_ms: Some(100.0),
        health: 1.0,
    };
    let candidates = vec![
        CandidateScoreInput {
            candidate_id: "a".into(),
            load_rate: 10,
            ..base.clone()
        },
        CandidateScoreInput {
            candidate_id: "b".into(),
            load_rate: 20,
            ..base.clone()
        },
        CandidateScoreInput {
            candidate_id: "c".into(),
            load_rate: 30,
            ..base.clone()
        },
        CandidateScoreInput {
            candidate_id: "d".into(),
            load_rate: 40,
            ..base.clone()
        },
    ];
    let top = select_top_k_candidates(&candidates, 4, &weights());

    let first = build_weighted_selection_order(&top, 7);
    let second = build_weighted_selection_order(&top, 7);

    let first_ids: Vec<_> = first
        .iter()
        .map(|c| c.input.candidate_id.as_str())
        .collect();
    let second_ids: Vec<_> = second
        .iter()
        .map(|c| c.input.candidate_id.as_str())
        .collect();
    assert_eq!(first_ids, second_ids);
}

#[test]
fn scheduler_weighted_order_can_vary_for_different_seeds() {
    let base = CandidateScoreInput {
        candidate_id: "base".into(),
        priority: 1,
        load_rate: 10,
        waiting_count: 0,
        max_waiting_count: 1,
        error_rate: 0.0,
        latency_ms: Some(100.0),
        min_latency_ms: Some(100.0),
        max_latency_ms: Some(100.0),
        health: 1.0,
    };
    let candidates = vec![
        CandidateScoreInput {
            candidate_id: "a".into(),
            load_rate: 10,
            ..base.clone()
        },
        CandidateScoreInput {
            candidate_id: "b".into(),
            load_rate: 20,
            ..base.clone()
        },
        CandidateScoreInput {
            candidate_id: "c".into(),
            load_rate: 30,
            ..base.clone()
        },
        CandidateScoreInput {
            candidate_id: "d".into(),
            load_rate: 40,
            ..base.clone()
        },
    ];
    let top = select_top_k_candidates(&candidates, 4, &weights());

    let first: Vec<_> = build_weighted_selection_order(&top, 1)
        .iter()
        .map(|c| c.input.candidate_id.clone())
        .collect();
    let second: Vec<_> = build_weighted_selection_order(&top, 999)
        .iter()
        .map(|c| c.input.candidate_id.clone())
        .collect();

    assert_ne!(
        first, second,
        "different seeds should be able to produce different attempt orders"
    );
}
