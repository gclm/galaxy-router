use std::collections::{BTreeMap, HashSet};

use crate::api::handlers::admin::channels::EndpointType;
use crate::error::proxy::ProxyError;
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

    // 第一遍：收集每个候选的原始指标（latency / active / error_rate / health）
    // 收集 latency/active 后算集内 min/max，供第二遍相对归一化
    let raw: Vec<(usize, Option<f64>, u32, f64, f64)> = candidates
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let status = states.get(&item.channel_id);
            let runtime = lb_state.runtime_stats(&item.channel_id);
            let runtime_latency = if runtime.avg_ttft_ms() > 0.0 {
                Some(runtime.avg_ttft_ms())
            } else if runtime.avg_latency_ms() > 0.0 {
                Some(runtime.avg_latency_ms())
            } else {
                None
            };
            let active = lb_state.capacity_manager().active_count(&item.channel_id);
            let health = runtime.health()
                * status
                    .map(|s| if s.is_available() { 1.0 } else { 0.0 })
                    .unwrap_or(1.0);
            (i, runtime_latency, active, runtime.error_rate(), health)
        })
        .collect();

    // 候选集内 min/max（latency 仅含有数据的；active 含全部）
    let lat_vals: Vec<f64> = raw.iter().filter_map(|(_, lat, _, _, _)| *lat).collect();
    let min_lat = lat_vals.iter().cloned().fold(f64::INFINITY, f64::min);
    let max_lat = lat_vals.iter().cloned().fold(0.0_f64, f64::max);
    let (min_lat, max_lat) = if lat_vals.is_empty() {
        (None, None)
    } else {
        (Some(min_lat), Some(max_lat))
    };

    let act_vals: Vec<f64> = raw.iter().map(|(_, _, a, _, _)| *a as f64).collect();
    let min_act = act_vals.iter().cloned().fold(f64::INFINITY, f64::min);
    let max_act = act_vals.iter().cloned().fold(0.0_f64, f64::max);
    let (min_act, max_act) = if act_vals.is_empty() {
        (None, None)
    } else {
        (Some(min_act), Some(max_act))
    };

    // 第二遍：构造 CandidateScoreInput（填入集内 min/max）
    let inputs: Vec<CandidateScoreInput> = raw
        .iter()
        .map(|(i, latency_ms, active, error_rate, health)| {
            let item = &candidates[*i];
            CandidateScoreInput {
                candidate_id: item.channel_id.clone(),
                active_concurrency: *active,
                min_active: min_act,
                max_active: max_act,
                error_rate: *error_rate,
                latency_ms: *latency_ms,
                min_latency_ms: min_lat,
                max_latency_ms: max_lat,
                health: *health,
            }
        })
        .collect();

    let top_k = inputs.len();
    select_top_k_candidates(&inputs, top_k, &SchedulerScoreWeights::default())
}

/// priority 硬分档 + health 自动降档排序（纯函数，便于单测）。
///
/// 输入 `(priority, ScoredCandidate)` 列表：priority 来自 group_items，ScoredCandidate
/// 来自档内打分。输出：按 priority 升序分档拼接（小值=高档先），health<=0 的候选沉到
/// 所有健康候选之后——health 是 EWMA 持续扣减值（runtime.health * is_available），归零
/// = 持续不健康，硬性排后，不被 latency/least_conn 补偿反超。
fn tier_and_demote(scored: Vec<(i32, ScoredCandidate)>) -> Vec<ScoredCandidate> {
    let mut tiers: BTreeMap<i32, Vec<ScoredCandidate>> = BTreeMap::new();
    for (prio, sc) in scored {
        tiers.entry(prio).or_default().push(sc);
    }
    let mut ordered = Vec::new();
    let mut demoted = Vec::new();
    for (_prio, group) in tiers {
        for sc in group {
            if sc.input.health <= 0.0 {
                demoted.push(sc);
            } else {
                ordered.push(sc);
            }
        }
    }
    ordered.extend(demoted);
    ordered
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
    _client_endpoint: &EndpointType,
    session_hash: Option<&str>,
) -> Result<Vec<RelayCandidate>, ProxyError> {
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();

    // 1. Sticky session
    if let Some(hash) = session_hash
        && let Some(channel_id) = state.lb_state.get_sticky_session(hash).await
        && let Ok(channel) = state.get_channel(&channel_id).await
        && channel.has_any_endpoint()
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
            // priority 硬分档：按 priority 值分档，每档内 score_candidates 打分（档内相对归一化）
            let mut tiers: BTreeMap<i32, Vec<&GroupItemInfo>> = BTreeMap::new();
            for item in &items {
                tiers.entry(item.priority).or_default().push(*item);
            }
            let mut scored_with_prio: Vec<(i32, ScoredCandidate)> = Vec::new();
            for (prio, tier_items) in &tiers {
                let tier_refs: Vec<&GroupItemInfo> = tier_items.to_vec();
                for sc in score_candidates(&state.lb_state, &tier_refs).await {
                    scored_with_prio.push((*prio, sc));
                }
            }
            // 分档拼接（高档先）+ health<=0 自动降级（沉到所有健康候选之后）
            let ordered = tier_and_demote(scored_with_prio);

            for sc in &ordered {
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

    fn scored(id: &str, health: f64, score: f64) -> ScoredCandidate {
        ScoredCandidate {
            input: CandidateScoreInput {
                candidate_id: id.into(),
                active_concurrency: 0,
                min_active: None,
                max_active: None,
                error_rate: 0.0,
                latency_ms: None,
                min_latency_ms: None,
                max_latency_ms: None,
                health,
            },
            score,
        }
    }

    /// priority 硬分档：高档（值小）排在低档之前
    #[test]
    fn tier_and_demote_high_priority_tier_first() {
        let out = tier_and_demote(vec![(2, scored("a", 1.0, 5.0)), (1, scored("b", 1.0, 3.0))]);
        assert_eq!(out[0].input.candidate_id, "b");
        assert_eq!(out[1].input.candidate_id, "a");
    }

    /// 同档内 health=0 沉底（即使 score 更高）
    #[test]
    fn tier_and_demote_unhealthy_sinks_within_tier() {
        let out = tier_and_demote(vec![
            (1, scored("healthy", 1.0, 1.0)),
            (1, scored("unhealthy", 0.0, 9.0)),
        ]);
        assert_eq!(out[0].input.candidate_id, "healthy");
        assert_eq!(out[1].input.candidate_id, "unhealthy");
    }

    /// 高档 health=0 沉到低档健康候选之后（跨档降级）
    #[test]
    fn tier_and_demote_unhealthy_sinks_below_lower_tier() {
        let out = tier_and_demote(vec![
            (1, scored("high-unhealthy", 0.0, 9.0)),
            (2, scored("low-healthy", 1.0, 1.0)),
        ]);
        assert_eq!(out[0].input.candidate_id, "low-healthy");
        assert_eq!(out[1].input.candidate_id, "high-unhealthy");
    }

    // ============================================================
    // M3-S1: scheduler scoring integration characterization
    // ============================================================

    /// 验证多因子打分：健康 + 低延迟渠道排在前面
    #[tokio::test]
    async fn scheduler_selection_integration_prefers_healthy_low_latency() {
        let lb = LoadBalancerState::new();
        lb.ensure_channel_status("ch-fast", 10, 3, 10).await;
        lb.ensure_channel_status("ch-slow", 10, 3, 10).await;

        // ch-fast: 低延迟、无错误
        lb.record_success("ch-fast", 50.0).await;

        // ch-slow: 高延迟、有失败记录
        lb.record_success("ch-slow", 500.0).await;
        lb.record_failure("ch-slow", false).await;
        lb.record_failure("ch-slow", false).await;

        let items = [
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

    /// 验证最小连接数：在途并发少的渠道得分更高（不依赖 max_concurrency 配置）
    #[tokio::test]
    async fn scheduler_selection_integration_least_conn_prefers_idle() {
        let lb = LoadBalancerState::new();
        lb.ensure_channel_status("ch-idle", 10, 3, 10).await;
        lb.ensure_channel_status("ch-busy", 10, 3, 10).await;

        // ch-busy: 占用 9 个并发槽（permit 持有不释放，保持 capacity active=9）
        let cm = lb.capacity_manager();
        let _permits: Vec<_> = (0..9)
            .map(|_| cm.try_acquire("ch-busy", 10).unwrap())
            .collect();

        let items = [
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
        assert_eq!(scored[1].input.active_concurrency, 9);
    }
}
