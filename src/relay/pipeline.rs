use axum::http::{HeaderMap, StatusCode};
use sqlx::SqlitePool;
use std::collections::HashSet;
use std::pin::Pin;

use crate::api::handlers::admin::channels::EndpointType;
use crate::metrics::recorder::redaction::sanitize_json_content;
use crate::metrics::recorder::save_request_record;
use crate::protocol::inbound::{Inbound, InboundError};
use crate::protocol::outbound::{Outbound, OutboundError};
use crate::protocol::stream_converter::StreamConverter;
use crate::relay::error::{ErrorFormat, ProxyError};
use crate::relay::run::RelayCandidate;
use crate::scheduler::scoring::{
    CandidateScoreInput, SchedulerScoreWeights, ScoredCandidate, select_top_k_candidates,
};
use crate::scheduler::state::LoadBalancerState;

use crate::relay::state::ProxyState;

#[derive(Debug, Clone, PartialEq)]
pub struct RelayPipelineRequest {
    pub client_endpoint: EndpointType,
    pub upstream_endpoint: EndpointType,
    pub requested_model: String,
    pub upstream_model: String,
    pub body: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreparedRelayRequest {
    pub client_endpoint: EndpointType,
    pub upstream_endpoint: EndpointType,
    pub requested_model: String,
    pub upstream_model: String,
    pub body: serde_json::Value,
    pub needs_conversion: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FinalizedRelayResponse {
    pub body: serde_json::Value,
    pub was_converted: bool,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RelayPipelineError {
    #[error("unsupported relay pipeline endpoint: {endpoint:?}")]
    UnsupportedEndpoint { endpoint: EndpointType },

    #[error("inbound conversion failed: {0}")]
    Inbound(String),

    #[error("outbound conversion failed: {0}")]
    Outbound(String),

    #[error("json conversion failed: {0}")]
    Json(String),
}

impl From<InboundError> for RelayPipelineError {
    fn from(value: InboundError) -> Self {
        Self::Inbound(value.to_string())
    }
}

impl From<OutboundError> for RelayPipelineError {
    fn from(value: OutboundError) -> Self {
        Self::Outbound(value.to_string())
    }
}

pub struct RelayPipeline;

impl RelayPipeline {
    pub fn prepare_request(
        request: RelayPipelineRequest,
    ) -> Result<PreparedRelayRequest, RelayPipelineError> {
        if request.client_endpoint != request.upstream_endpoint {
            return Err(RelayPipelineError::UnsupportedEndpoint {
                endpoint: request.upstream_endpoint,
            });
        }

        let mut body = request.body;
        rewrite_model(&mut body, &request.upstream_model);

        Ok(PreparedRelayRequest {
            client_endpoint: request.client_endpoint,
            upstream_endpoint: request.upstream_endpoint,
            requested_model: request.requested_model,
            upstream_model: request.upstream_model,
            body,
            needs_conversion: false,
        })
    }

    pub async fn prepare_request_async(
        request: RelayPipelineRequest,
    ) -> Result<PreparedRelayRequest, RelayPipelineError> {
        if request.client_endpoint == request.upstream_endpoint {
            return Self::prepare_request(request);
        }

        let body_bytes = serde_json::to_vec(&request.body)
            .map_err(|e| RelayPipelineError::Json(e.to_string()))?;
        let mut llm_request = inbound_for(&request.client_endpoint)?
            .transform_request(&body_bytes, &HeaderMap::new())
            .await?;
        llm_request.model = request.upstream_model.clone();

        let upstream_body_bytes =
            outbound_for(&request.upstream_endpoint)?.transform_request(&llm_request)?;
        let upstream_body = serde_json::from_slice(&upstream_body_bytes)
            .map_err(|e| RelayPipelineError::Json(e.to_string()))?;

        Ok(PreparedRelayRequest {
            client_endpoint: request.client_endpoint,
            upstream_endpoint: request.upstream_endpoint,
            requested_model: request.requested_model,
            upstream_model: request.upstream_model,
            body: upstream_body,
            needs_conversion: true,
        })
    }

    pub fn finalize_response(
        client_endpoint: EndpointType,
        upstream_endpoint: EndpointType,
        body: serde_json::Value,
    ) -> Result<FinalizedRelayResponse, RelayPipelineError> {
        if client_endpoint != upstream_endpoint {
            return Err(RelayPipelineError::UnsupportedEndpoint {
                endpoint: upstream_endpoint,
            });
        }

        Ok(FinalizedRelayResponse {
            body,
            was_converted: false,
        })
    }

    pub async fn finalize_response_async(
        client_endpoint: EndpointType,
        upstream_endpoint: EndpointType,
        body: serde_json::Value,
        status: u16,
    ) -> Result<FinalizedRelayResponse, RelayPipelineError> {
        if client_endpoint == upstream_endpoint {
            return Self::finalize_response(client_endpoint, upstream_endpoint, body);
        }

        let body_bytes =
            serde_json::to_vec(&body).map_err(|e| RelayPipelineError::Json(e.to_string()))?;
        let llm_response = outbound_for(&upstream_endpoint)?
            .transform_response(&body_bytes, status)
            .await?;
        let client_body_bytes = inbound_for(&client_endpoint)?.transform_response(&llm_response)?;
        let client_body = serde_json::from_slice(&client_body_bytes)
            .map_err(|e| RelayPipelineError::Json(e.to_string()))?;

        Ok(FinalizedRelayResponse {
            body: client_body,
            was_converted: true,
        })
    }

    /// 流式转换是否需要跨协议转换
    #[allow(dead_code)]
    pub fn needs_stream_conversion(
        client_endpoint: &EndpointType,
        upstream_endpoint: &EndpointType,
    ) -> bool {
        client_endpoint != upstream_endpoint
    }

    /// 创建客户端协议的流式转换器（conversion 路径使用）
    ///
    /// passthrough 路径（相同端点）返回 None
    pub fn create_stream_converter(
        client_endpoint: &EndpointType,
        upstream_endpoint: &EndpointType,
    ) -> Result<Option<Box<dyn StreamConverter>>, RelayPipelineError> {
        if client_endpoint == upstream_endpoint {
            return Ok(None);
        }
        let converter = inbound_for(client_endpoint)?.create_stream_converter();
        Ok(Some(converter))
    }

    /// 解码上游 SSE 事件为统一的 LlmStreamResponse
    ///
    /// 返回 None 表示事件被忽略（如心跳/注释行）
    pub fn decode_stream_event(
        upstream_endpoint: &EndpointType,
        event_bytes: &[u8],
    ) -> Result<Option<crate::protocol::model::LlmStreamResponse>, RelayPipelineError> {
        let result = outbound_for(upstream_endpoint)?
            .transform_stream_event(event_bytes)
            .map_err(|e| RelayPipelineError::Outbound(e.to_string()))?;
        Ok(result)
    }
}

fn rewrite_model(body: &mut serde_json::Value, upstream_model: &str) {
    if let Some(obj) = body.as_object_mut() {
        obj.insert(
            "model".to_string(),
            serde_json::Value::String(upstream_model.to_string()),
        );
    }
}

fn inbound_for(endpoint: &EndpointType) -> Result<&'static dyn Inbound, RelayPipelineError> {
    crate::protocol::inbound::inbound_for(endpoint).ok_or(RelayPipelineError::UnsupportedEndpoint {
        endpoint: endpoint.clone(),
    })
}

fn outbound_for(endpoint: &EndpointType) -> Result<&'static dyn Outbound, RelayPipelineError> {
    crate::protocol::outbound::outbound_for(endpoint).ok_or(
        RelayPipelineError::UnsupportedEndpoint {
            endpoint: endpoint.clone(),
        },
    )
}

// =====================================================================
// Orchestration, candidates, validation, and entry functions (migrated from state.rs)
// =====================================================================

/// 获取出站转换器（静态引用，避免堆分配）
pub fn get_outbound(endpoint_type: &EndpointType) -> &'static dyn Outbound {
    crate::protocol::outbound::outbound_for(endpoint_type)
        .unwrap_or(&crate::protocol::outbound::openai_chat::OpenAiChatOutbound)
}

/// M3-S1: 将候选渠道转换为 CandidateScoreInput 并用 scheduler 多因子打分排序
async fn score_candidates(
    lb_state: &LoadBalancerState,
    candidates: &[&crate::scheduler::selector::GroupItemInfo],
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
    use crate::scheduler::selector::GroupItemInfo;

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

/// 验证字符串可作为 HTTP header value
/// 用于在保存上游 API Key 时一次性拦截含 CRLF / 控制字符的输入，
/// 避免转发时 `HeaderValue::from_str(...).unwrap()` panic。
pub(crate) fn validate_header_value(s: &str) -> Result<(), String> {
    reqwest::header::HeaderValue::from_str(s)
        .map(|_| ())
        .map_err(|e| format!("含非法 header 字符 ({e})"))
}

/// 检查 API Key 的预算额度（月/日）
async fn check_budget(pool: &SqlitePool, key_id: &str) -> Result<(), String> {
    let limit: Option<(f64, f64)> = sqlx::query_as(
        "SELECT monthly_limit_usd, daily_limit_usd FROM budget_limits WHERE api_key_id = ? AND enabled = 1",
    )
    .bind(key_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("查询预算失败: {}", e))?;

    let Some((monthly_limit, daily_limit)) = limit else {
        return Ok(()); // 无预算限制
    };

    // 查询累计消费
    let (monthly_cost, daily_cost): (f64, f64) = sqlx::query_as(
        r#"SELECT
            COALESCE(SUM(COALESCE(cost, 0)), 0),
            COALESCE(SUM(CASE WHEN date(created_at) = date('now') THEN COALESCE(cost, 0) ELSE 0 END), 0)
        FROM usage_logs WHERE api_key_id = ?"#,
    )
    .bind(key_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("查询消费失败: {}", e))?;

    if daily_limit > 0.0 && daily_cost >= daily_limit {
        return Err(format!(
            "日预算已耗尽: ${:.2}/${:.2}",
            daily_cost, daily_limit
        ));
    }
    if monthly_limit > 0.0 && monthly_cost >= monthly_limit {
        return Err(format!(
            "月预算已耗尽: ${:.2}/${:.2}",
            monthly_cost, monthly_limit
        ));
    }

    Ok(())
}

/// 验证 API Key 是否有权访问目标模型（三段式：403→404→503）
async fn validate_model_access(
    pool: &SqlitePool,
    key_id: &str,
    model: &str,
    allowed_groups: &str,
) -> Result<(), ProxyError> {
    // === 第一关：检查 allowed_groups / supported_models ===
    // allowed_groups 非空时，检查请求的 model 是否在允许的分组列表中
    if !allowed_groups.is_empty() {
        let allowed: Vec<&str> = allowed_groups
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if !allowed.is_empty() && !allowed.contains(&model) {
            return Err(ProxyError::ModelNotSupported(format!(
                "API Key 无权访问模型: {}",
                model
            )));
        }
    } else {
        // allowed_groups 为空时回退到 supported_models（兼容旧逻辑）
        let supported = sqlx::query_scalar::<_, String>(
            "SELECT supported_models FROM api_keys WHERE id = ? AND enabled = 1",
        )
        .bind(key_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        if let Some(models_str) = supported
            && !models_str.is_empty()
        {
            let allowed =
                crate::api::handlers::admin::api_keys::parse_supported_models(&models_str);
            if !allowed.iter().any(|m| m == model) {
                return Err(ProxyError::ModelNotSupported(format!(
                    "API Key 无权访问模型: {}",
                    model
                )));
            }
        }
    }

    // === 第二关：检查分组是否存在 ===
    // 仅在 allowed_groups 非空时严格检查（Octopus 策略：明确指定了分组才校验）
    // allowed_groups 为空时跳过，由后续 Relay candidate 构建检查渠道可用性
    if !allowed_groups.is_empty() {
        let group_exists: bool = sqlx::query_scalar::<_, i32>(
            "SELECT COUNT(*) FROM groups WHERE name = ? AND enabled = 1",
        )
        .bind(model)
        .fetch_one(pool)
        .await
        .map(|c| c > 0)
        .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        if !group_exists {
            return Err(ProxyError::ModelNotFound(format!("模型不存在: {}", model)));
        }
    }

    // === 第三关：分组内是否有可用渠道（延迟到 Relay candidate 构建时检查）===
    Ok(())
}

/// 非流式代理请求（支持重试和排队）
pub async fn proxy_request(
    state: &ProxyState,
    request_id: &str,
    api_key_id: Option<&str>,
    headers: &HeaderMap,
    body: &serde_json::Value,
    client_endpoint: &EndpointType,
) -> Result<crate::relay::state::ProxySuccess, ProxyError> {
    let _permit = if let Some(queue) = &state.queue {
        Some(
            queue
                .acquire()
                .await
                .map_err(|e| ProxyError::RequestError(format!("排队失败: {}", e)))?,
        )
    } else {
        None
    };

    let model = body["model"].as_str().unwrap_or("unknown").to_string();
    let request_content = serde_json::to_string(&body)
        .ok()
        .map(|c| sanitize_json_content(&c));
    let user_agent = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let session_hash = headers
        .get("x-session-hash")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let sticky_channel_id = if let Some(hash) = session_hash.as_deref() {
        state.lb_state.get_sticky_session(hash).await
    } else {
        None
    };

    // 1. 构建候选列表（sticky + scored group items）
    let candidates =
        match build_relay_candidates(state, &model, client_endpoint, session_hash.as_deref()).await
        {
            Ok(c) => c,
            Err(e) => {
                save_request_record(
                    state,
                    Some(request_id.to_string()),
                    api_key_id,
                    None,
                    &model,
                    request_content,
                    None,
                    &[],
                    None,
                    false,
                    user_agent,
                    Some(&e),
                )
                .await;
                return Err(e);
            }
        };

    // 2. 创建 executor + RelayRun
    let executor = crate::relay::executor::ProxyRelayExecutor::new(
        state.clone(),
        headers.clone(),
        body.clone(),
        client_endpoint.clone(),
        api_key_id.map(|s| s.to_string()),
    );

    let capacity = state.lb_state.capacity_manager();
    let run = crate::relay::run::RelayRun::new(capacity, executor.clone());
    let relay_request = crate::relay::run::RelayRequest::new(&model);

    // 3. 执行
    let candidate_snapshot = candidates.clone();
    let outcome = run.execute(relay_request, candidates).await;
    let attempt_stats = executor.take_attempt_stats();

    // 4. 记录日志 + 返回结果
    if let Some(response_body) = outcome.response {
        if let Some(channel_id) = outcome.selected_channel_id.as_deref() {
            let selected_was_sticky = candidate_snapshot
                .iter()
                .any(|c| c.channel_id == channel_id && c.sticky);
            state.lb_state.record_scheduler_selection(
                channel_id,
                selected_was_sticky,
                sticky_channel_id.as_deref(),
            );
            if let Some(hash) = session_hash.as_deref() {
                state.lb_state.set_sticky_session(hash, channel_id).await;
            }
        }
        save_request_record(
            state,
            Some(request_id.to_string()),
            api_key_id,
            None,
            &model,
            request_content,
            Some(response_body.clone()),
            &attempt_stats,
            None,
            false,
            user_agent,
            None,
        )
        .await;

        Ok(crate::relay::state::ProxySuccess {
            status: StatusCode::OK,
            body: response_body.into_bytes(),
        })
    } else {
        let error = ProxyError::from_relay_outcome(&outcome);
        save_request_record(
            state,
            Some(request_id.to_string()),
            api_key_id,
            None,
            &model,
            request_content,
            None,
            &attempt_stats,
            None,
            false,
            user_agent,
            Some(&error),
        )
        .await;
        Err(error)
    }
}

/// 流式代理请求（支持重试和排队）
pub async fn proxy_stream(
    state: &ProxyState,
    request_id: &str,
    api_key_id: Option<&str>,
    headers: &HeaderMap,
    body: &serde_json::Value,
    client_endpoint: &EndpointType,
) -> Result<
    (
        StatusCode,
        Pin<
            Box<
                dyn futures::Stream<Item = Result<axum::body::Bytes, std::convert::Infallible>>
                    + Send
                    + 'static,
            >,
        >,
        String,
    ),
    ProxyError,
> {
    let queue_permit = if let Some(queue) = &state.queue {
        Some(
            queue
                .acquire()
                .await
                .map_err(|e| ProxyError::RequestError(format!("排队失败: {}", e)))?,
        )
    } else {
        None
    };

    let model = body["model"].as_str().unwrap_or("unknown").to_string();
    let request_content = serde_json::to_string(&body).ok();
    let user_agent = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let session_hash = headers
        .get("x-session-hash")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let sticky_channel_id = if let Some(hash) = session_hash.as_deref() {
        state.lb_state.get_sticky_session(hash).await
    } else {
        None
    };

    let candidates =
        match build_relay_candidates(state, &model, client_endpoint, session_hash.as_deref()).await
        {
            Ok(c) => c,
            Err(e) => {
                save_request_record(
                    state,
                    Some(request_id.to_string()),
                    api_key_id,
                    None,
                    &model,
                    request_content,
                    None,
                    &[],
                    None,
                    true,
                    user_agent,
                    Some(&e),
                )
                .await;
                return Err(e);
            }
        };

    let executor = crate::relay::stream_executor::ProxyStreamRelayExecutor::new(
        state.clone(),
        request_id.to_string(),
        headers.clone(),
        body.clone(),
        client_endpoint.clone(),
        api_key_id.map(|s| s.to_string()),
        queue_permit,
    );
    let run =
        crate::relay::run::RelayStreamRun::new(state.lb_state.capacity_manager(), executor.clone());
    let candidate_snapshot = candidates.clone();
    let outcome = run
        .execute(crate::relay::run::RelayRequest::new(&model), candidates)
        .await;

    if let Some(success) = outcome.response {
        if let Some(channel_id) = outcome.selected_channel_id.as_deref() {
            let selected_was_sticky = candidate_snapshot
                .iter()
                .any(|c| c.channel_id == channel_id && c.sticky);
            state.lb_state.record_scheduler_selection(
                channel_id,
                selected_was_sticky,
                sticky_channel_id.as_deref(),
            );
            if let Some(hash) = session_hash.as_deref() {
                state.lb_state.set_sticky_session(hash, channel_id).await;
            }
        }
        return Ok((success.status, success.stream, success.content_type));
    }

    let attempt_stats = executor.take_attempt_stats();
    let error = ProxyError::from_relay_stream_outcome(&outcome);
    save_request_record(
        state,
        Some(request_id.to_string()),
        api_key_id,
        None,
        &model,
        request_content,
        None,
        &attempt_stats,
        None,
        true,
        user_agent,
        Some(&error),
    )
    .await;
    Err(error)
}

/// 统一代理请求入口（供各 handler 调用）
pub async fn handle_proxy_request(
    state: &ProxyState,
    auth: crate::api::middleware::ApiKeyAuth,
    headers: HeaderMap,
    body: serde_json::Value,
    client_endpoint: &EndpointType,
    error_format: &ErrorFormat,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let request_id = crate::api::response::generate_id();
    let model = body["model"].as_str().unwrap_or("unknown");
    let is_stream = body["stream"].as_bool().unwrap_or(false);
    let api_key_id = Some(auth.key_id.as_str());

    // 速率限制检查（RPM + TPM）
    if let Err(retry_after) = state
        .rate_limiter
        .check_rpm(&auth.key_id, auth.rate_limit_rpm)
        .await
    {
        return crate::relay::error::format_rate_limit_error(retry_after, "RPM", auth.rate_limit_rpm, error_format);
    }
    if let Err(retry_after) = state
        .rate_limiter
        .check_tpm(&auth.key_id, auth.rate_limit_tpm)
        .await
    {
        return crate::relay::error::format_rate_limit_error(retry_after, "TPM", auth.rate_limit_tpm, error_format);
    }

    // 预算检查（月/日额度）
    if let Err(msg) = check_budget(&state.pool, &auth.key_id).await {
        return crate::relay::error::format_budget_error(&msg, error_format);
    }

    // 验证 API Key 是否有权访问目标模型（三段式：403→404→503）
    if let Err(e) =
        validate_model_access(&state.pool, &auth.key_id, model, &auth.allowed_groups).await
    {
        return crate::relay::error::format_proxy_error(e, error_format);
    }

    if is_stream {
        match proxy_stream(
            state,
            &request_id,
            api_key_id,
            &headers,
            &body,
            client_endpoint,
        )
        .await
        {
            Ok((status, stream, content_type)) => axum::response::Response::builder()
                .status(status)
                .header("Content-Type", content_type)
                .header("Cache-Control", "no-cache")
                .header("Connection", "keep-alive")
                .header("X-Request-ID", &request_id)
                .body(axum::body::Body::from_stream(stream))
                .expect("static headers + StatusCode from upstream are valid Response inputs")
                .into_response(),
            Err(e) => crate::relay::error::format_proxy_error(e, error_format),
        }
    } else {
        match proxy_request(
            state,
            &request_id,
            api_key_id,
            &headers,
            &body,
            client_endpoint,
        )
        .await
        {
            Ok(result) => axum::response::Response::builder()
                .status(result.status)
                .header("Content-Type", "application/json")
                .header("X-Request-ID", &request_id)
                .body(axum::body::Body::from(result.body))
                .expect("static Content-Type + StatusCode are valid Response inputs")
                .into_response(),
            Err(e) => crate::relay::error::format_proxy_error(e, error_format),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::error::is_key_retryable_upstream_error;
    use crate::scheduler::selector::GroupItemInfo;
    use crate::scheduler::state::LoadBalancerState;

    #[test]
    fn validate_header_value_accepts_normal_and_rejects_crlf() {
        assert!(validate_header_value("sk-abc.123_OK-9").is_ok());
        assert!(validate_header_value("sk-abc/def+ghi=").is_ok());
        assert!(validate_header_value("sk-abc").is_ok());
        assert!(validate_header_value("sk-abc\r\nfoo").is_err());
        assert!(validate_header_value("sk-abc\0").is_err());
        assert!(validate_header_value("sk-abc\x7f").is_err());
    }

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

    // ============================================================
    // M1-S2: Pipeline conversion characterization
    // ============================================================

    /// passthrough 直通路径保持响应体原样
    #[test]
    fn response_pipeline_conversion_passthrough_preserves_body() {
        let upstream_body = serde_json::json!({
            "id": "chatcmpl-42",
            "object": "chat.completion",
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "hello"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7}
        });

        let finalized = RelayPipeline::finalize_response(
            EndpointType::OpenAiChat,
            EndpointType::OpenAiChat,
            upstream_body.clone(),
        )
        .expect("passthrough finalize should succeed");

        assert!(!finalized.was_converted);
        assert_eq!(finalized.body, upstream_body);
    }

    /// 转换路径将 Anthropic 响应转为 OpenAI Chat 格式
    #[tokio::test]
    async fn response_pipeline_conversion_anthropic_to_openai_chat() {
        let anthropic_response = serde_json::json!({
            "id": "msg_conv",
            "type": "message",
            "role": "assistant",
            "model": "claude-3-5-sonnet",
            "content": [{"type": "text", "text": "converted reply"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 8, "output_tokens": 3}
        });

        let finalized = RelayPipeline::finalize_response_async(
            EndpointType::OpenAiChat,
            EndpointType::Anthropic,
            anthropic_response,
            200,
        )
        .await
        .expect("conversion finalize should succeed");

        assert!(finalized.was_converted);
        assert_eq!(finalized.body["object"], "chat.completion");
        assert_eq!(finalized.body["choices"][0]["message"]["content"], "converted reply");
        assert_eq!(finalized.body["choices"][0]["finish_reason"], "stop");
    }

    /// 转换路径将 OpenAI Responses 响应转为 OpenAI Chat 格式
    #[tokio::test]
    async fn response_pipeline_conversion_responses_to_chat() {
        let responses_body = serde_json::json!({
            "id": "resp_conv",
            "object": "response",
            "model": "gpt-4o",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "hi from responses"}]
            }],
            "usage": {"input_tokens": 4, "output_tokens": 2, "total_tokens": 6}
        });

        let finalized = RelayPipeline::finalize_response_async(
            EndpointType::OpenAiChat,
            EndpointType::OpenAiResponse,
            responses_body,
            200,
        )
        .await
        .expect("responses→chat conversion should succeed");

        assert!(finalized.was_converted);
        assert_eq!(finalized.body["object"], "chat.completion");
        assert_eq!(finalized.body["choices"][0]["message"]["content"], "hi from responses");
    }

    /// 相同端点 finalize_response_async 走 passthrough
    #[tokio::test]
    async fn response_pipeline_conversion_same_endpoint_uses_fast_path() {
        let body = serde_json::json!({"id": "test", "result": "ok"});
        let finalized = RelayPipeline::finalize_response_async(
            EndpointType::Anthropic,
            EndpointType::Anthropic,
            body.clone(),
            200,
        )
        .await
        .expect("same-endpoint async finalize should succeed");

        assert!(!finalized.was_converted);
        assert_eq!(finalized.body, body);
    }

    // ============================================================
    // P4.4a: Stream pipeline characterization
    // ============================================================

    #[test]
    fn stream_pipeline_needs_conversion_true_for_different_endpoints() {
        assert!(RelayPipeline::needs_stream_conversion(
            &EndpointType::OpenAiChat,
            &EndpointType::Anthropic
        ));
    }

    #[test]
    fn stream_pipeline_needs_conversion_false_for_same_endpoint() {
        assert!(!RelayPipeline::needs_stream_conversion(
            &EndpointType::OpenAiChat,
            &EndpointType::OpenAiChat
        ));
    }

    #[test]
    fn stream_pipeline_create_converter_returns_none_for_passthrough() {
        let converter = RelayPipeline::create_stream_converter(
            &EndpointType::OpenAiChat,
            &EndpointType::OpenAiChat,
        )
        .expect("passthrough should succeed");
        assert!(converter.is_none());
    }

    #[test]
    fn stream_pipeline_create_converter_returns_some_for_conversion() {
        let converter = RelayPipeline::create_stream_converter(
            &EndpointType::OpenAiChat,
            &EndpointType::Anthropic,
        )
        .expect("anthropic→chat should succeed");
        assert!(converter.is_some());
    }

    /// 验证 decode_stream_event 能解析 Anthropic content_block_delta 事件
    #[test]
    fn stream_pipeline_decode_anthropic_content_delta() {
        let event = b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\n";

        let llm_event = RelayPipeline::decode_stream_event(&EndpointType::Anthropic, event)
            .expect("decode should succeed")
            .expect("should produce LlmStreamResponse");

        if let Some(choice) = llm_event.first_choice() {
            if let Some(crate::protocol::model::Content::Text(t)) = &choice.delta.content {
                assert_eq!(t, "Hi");
            } else {
                panic!("expected text content in delta");
            }
        } else {
            panic!("expected first_choice in decoded event");
        }
    }
}
