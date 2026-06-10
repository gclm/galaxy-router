use std::collections::HashSet;

use crate::api::handlers::admin::channels::EndpointType;
use crate::relay::error::ProxyError;
use crate::relay::run::RelayCandidate;
use crate::relay::state::ProxyState;
use crate::scheduler::scoring::{
    CandidateScoreInput, SchedulerScoreWeights, ScoredCandidate, select_top_k_candidates,
};
use crate::scheduler::selector::GroupItemInfo;
use crate::scheduler::state::LoadBalancerState;

/// M3-S1: 将候选渠道转换为 CandidateScoreInput 并用 scheduler 多因子打分排序
async fn score_candidates(
    lb_state: &LoadBalancerState,
    candidates: &[&GroupItemInfo],
) -> Vec<ScoredCandidate> {
    let states = lb_state.channel_states.read().await;

    let inputs: Vec<CandidateScoreInput> = candidates
        .iter()
        .map(|item| {
            let status = states.get(&item.channel_id);
            let runtime = lb_state.runtime_stats(&item.channel_id);
            let runtime_latency = if runtime.avg_ttft_ms() > 0.0 {
                Some(runtime.avg_ttft_ms())
            } else if runtime.avg_latency_ms() > 0.0 {
                Some(runtime.avg_latency_ms())
            } else {
                None
            };
            CandidateScoreInput {
                candidate_id: item.channel_id.clone(),
                priority: item.priority,
                load_rate: status.map(|s| s.load_rate()).unwrap_or(0),
                waiting_count: 0,
                max_waiting_count: 0,
                error_rate: runtime.error_rate(),
                latency_ms: runtime_latency,
                min_latency_ms: None,
                max_latency_ms: None,
                health: runtime.health()
                    * status
                        .map(|s| if s.is_available() { 1.0 } else { 0.0 })
                        .unwrap_or(1.0),
            }
        })
        .collect();

    let top_k = inputs.len().min(3);
    select_top_k_candidates(&inputs, top_k, &SchedulerScoreWeights::default())
}

/// 构建有序候选列表（sticky + scored group items）
///
/// 流程：
/// 1. 检查 sticky session → 加入候选（sticky=true, score=最高）
/// 2. 查找分组 → 获取 group_items → score_candidates 打分排序
/// 3. 构建 Vec<RelayCandidates>（sticky 在前，按 score 降序）
pub(crate) async fn build_relay_candidates(
    state: &ProxyState,
    model: &str,
    client_endpoint: &EndpointType,
    session_hash: Option<&str>,
) -> Result<Vec<RelayCandidate>, ProxyError> {
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();

    // 1. Sticky session
    if let Some(hash) = session_hash
        && let Some(channel_id) = state.lb_state.get_sticky_session(hash).await
        && let Ok(channel) = state.get_channel(&channel_id).await
        && channel.find_endpoint(client_endpoint).is_some()
    {
        seen.insert(channel_id.clone());
        candidates.push(RelayCandidate {
            channel_id: channel_id.clone(),
            channel_name: channel.name.clone(),
            max_concurrency: channel.max_concurrency,
            score: 100.0,
            sticky: true,
            target_model: model.to_string(),
            group_id: None,
        });
    }

    // 2. 分组候选（精确端点匹配）
    let group = match state.find_group_by_name(model).await? {
        Some(g) => Some(g),
        None => state.find_group_by_regex(model).await?,
    };

    if let Some(ref group) = group {
        let items: Vec<&GroupItemInfo> = group
            .items
            .iter()
            .filter(|item| !seen.contains(&item.channel_id))
            .collect();

        if !items.is_empty() {
            let scored = score_candidates(&state.lb_state, &items).await;
            for sc in &scored {
                if seen.insert(sc.input.candidate_id.clone()) {
                    let max_concurrency = state
                        .get_channel(&sc.input.candidate_id)
                        .await
                        .map(|ch| ch.max_concurrency)
                        .unwrap_or(0);
                    let target_model = items
                        .iter()
                        .find(|it| it.channel_id == sc.input.candidate_id)
                        .map(|it| it.model_name.clone())
                        .unwrap_or_else(|| model.to_string());

                    candidates.push(RelayCandidate {
                        channel_id: sc.input.candidate_id.clone(),
                        channel_name: String::new(),
                        max_concurrency,
                        score: sc.score,
                        sticky: false,
                        target_model,
                        group_id: Some(group.id.clone()),
                    });
                }
            }
        }
    }

    // 3. 候选为空时区分 "模型不存在" vs "渠道不可用"
    if candidates.is_empty() {
        return if group.is_some() {
            Err(ProxyError::NoAvailableChannel(format!(
                "模型 {} 的所有渠道不可用",
                model
            )))
        } else {
            Err(ProxyError::ModelNotFound(format!("模型不存在: {}", model)))
        };
    }

    Ok(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============================================================
    // M3-S1: scheduler scoring integration characterization
    // ============================================================

    /// 验证多因子打分：健康 + 低延迟渠道排在前面
    #[tokio::test]
    async fn scheduler_selection_integration_prefers_healthy_low_latency() {
        let lb = LoadBalancerState::new();
        lb.ensure_channel_status("ch-fast", 10).await;
        lb.ensure_channel_status("ch-slow", 10).await;

        // ch-fast: 低延迟、无错误
        lb.record_success("ch-fast", 50.0).await;

        // ch-slow: 高延迟、有失败记录
        lb.record_success("ch-slow", 500.0).await;
        lb.record_failure("ch-slow", false).await;
        lb.record_failure("ch-slow", false).await;

        let items = vec![
            GroupItemInfo {
                channel_id: "ch-fast".into(),
                model_name: "gpt-4o".into(),
                priority: 1,
                weight: 10,
            },
            GroupItemInfo {
                channel_id: "ch-slow".into(),
                model_name: "gpt-4o".into(),
                priority: 1,
                weight: 10,
            },
        ];
        let refs: Vec<&GroupItemInfo> = items.iter().collect();

        let scored = score_candidates(&lb, &refs).await;

        assert_eq!(scored.len(), 2);
        assert_eq!(scored[0].input.candidate_id, "ch-fast");
        assert!(
            scored[0].score > scored[1].score,
            "healthy candidate should score higher"
        );
    }

    /// 验证容量惩罚：高负载渠道得分更低
    #[tokio::test]
    async fn scheduler_selection_integration_full_capacity_load_penalty() {
        let lb = LoadBalancerState::new();
        lb.ensure_channel_status("ch-idle", 10).await;
        lb.ensure_channel_status("ch-busy", 10).await;

        // ch-busy: 9/10 负载
        for _ in 0..9 {
            lb.increment_active("ch-busy").await;
        }

        let items = vec![
            GroupItemInfo {
                channel_id: "ch-idle".into(),
                model_name: "gpt-4o".into(),
                priority: 1,
                weight: 10,
            },
            GroupItemInfo {
                channel_id: "ch-busy".into(),
                model_name: "gpt-4o".into(),
                priority: 1,
                weight: 10,
            },
        ];
        let refs: Vec<&GroupItemInfo> = items.iter().collect();

        let scored = score_candidates(&lb, &refs).await;

        assert_eq!(scored[0].input.candidate_id, "ch-idle");
        assert!(
            scored[0].score > scored[1].score,
            "idle candidate should score higher than busy"
        );
        assert_eq!(scored[1].input.load_rate, 90);
    }

    /// 验证优先级：低优先级数值（高优先）的渠道排在前面
    #[tokio::test]
    async fn scheduler_selection_integration_priority_takes_precedence() {
        let lb = LoadBalancerState::new();

        let items = vec![
            GroupItemInfo {
                channel_id: "ch-low-prio".into(),
                model_name: "gpt-4o".into(),
                priority: 5,
                weight: 10,
            },
            GroupItemInfo {
                channel_id: "ch-high-prio".into(),
                model_name: "gpt-4o".into(),
                priority: 1,
                weight: 5,
            },
        ];
        let refs: Vec<&GroupItemInfo> = items.iter().collect();

        let scored = score_candidates(&lb, &refs).await;

        assert_eq!(scored[0].input.candidate_id, "ch-high-prio");
        assert!(
            scored[0].score > scored[1].score,
            "high priority (low value) should score higher"
        );
    }
}
