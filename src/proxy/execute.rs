use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};

use super::prepare::{
    estimate_tokens, extract_request_text, extract_response_text, extract_usage,
    prepare_proxy_request,
};
use super::selection::SelectionResult;
use super::{ProxyError, ProxyState, ProxySuccess};
use crate::api::handlers::admin::channels::EndpointType;
use crate::proxy::sse::{
    apply_sse_usage, collect_sse_content, extract_error_from_sse, extract_usage_from_sse,
    find_sse_boundary, format_stream_error_event, sanitize_upstream_error,
};
use crate::proxy::thinking_normalizer::{PassthroughNormalizer, ThinkingTagExtractor};
use crate::relay::pipeline::RelayPipeline;
use crate::scheduler::trace::{AttemptStatus, AttemptTrace};
use crate::stats::redaction::sanitize_json_content;

/// 从渠道 extras JSON Map 读取 `extras.thinking.<key>` 布尔开关
fn thinking_flag(extras: &Option<serde_json::Map<String, serde_json::Value>>, key: &str) -> bool {
    extras
        .as_ref()
        .and_then(|e| e.get("thinking"))
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// 单次尝试的统计信息
pub(crate) struct AttemptStats {
    channel_id: String,
    target_model: String,
    upstream_endpoint: EndpointType,
    needs_conversion: bool,
    latency_ms: i64,
    status_code: u16,
    input_tokens: i32,
    output_tokens: i32,
    cache_read: i32,
    cache_creation: i32,
    cost: Option<f64>,
    error_message: Option<String>,
    upstream_key_hint: String,
}

impl AttemptStats {
    /// 转换为 scheduler::trace::AttemptTrace（M2-S1 adapter）
    /// 桥接代码：RelayRun 接入真实 proxy 后将直接使用 AttemptTrace
    #[allow(dead_code)]
    pub(super) fn to_trace(
        &self,
        attempt_no: u32,
        requested_model: &str,
        client_endpoint: &EndpointType,
    ) -> AttemptTrace {
        AttemptTrace {
            attempt_no,
            channel_id: Some(self.channel_id.clone()),
            channel_name: None,
            upstream_key_hint: Some(self.upstream_key_hint.clone()),
            requested_model: requested_model.to_string(),
            upstream_model: Some(self.target_model.clone()),
            client_endpoint: Some(client_endpoint.as_str().to_string()),
            upstream_endpoint: Some(self.upstream_endpoint.as_str().to_string()),
            status: if (200..400).contains(&self.status_code) {
                AttemptStatus::Success
            } else {
                AttemptStatus::Failed
            },
            reason: self.error_message.clone(),
            duration_ms: Some(self.latency_ms),
            queue_wait_ms: None,
            sticky: false,
            score: None,
        }
    }
}

/// RAII guard：确保函数退出时自动递减活跃请求数
struct ActiveRequestGuard {
    state: ProxyState,
    channel_id: String,
}

impl Drop for ActiveRequestGuard {
    fn drop(&mut self) {
        let state = self.state.clone();
        let channel_id = self.channel_id.clone();
        // spawn 一个任务来执行 async decrement，避免在 Drop 中 await
        tokio::spawn(async move {
            state.lb_state.decrement_active(&channel_id).await;
        });
    }
}

/// 仅在未被 decrement 过时执行一次 decrement（用于流式请求的错误路径）
fn decrement_active_once(
    state: &ProxyState,
    channel_id: &str,
    done: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    if done
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
        )
        .is_ok()
    {
        let state = state.clone();
        let channel_id = channel_id.to_string();
        tokio::spawn(async move {
            state.lb_state.decrement_active(&channel_id).await;
        });
    }
}

impl crate::stats::recorder::RequestRecord {
    /// 从最后一次尝试构造完整记录
    #[allow(clippy::too_many_arguments)]
    fn from_last_attempt(
        last: &AttemptStats,
        request_id: Option<String>,
        api_key_id: Option<&str>,
        group_id: Option<&str>,
        model: &str,
        request_content: Option<String>,
        response_content: Option<String>,
        channel_attempts: Vec<crate::stats::recorder::ChannelAttempt>,
        ttft_ms: Option<i32>,
        is_stream: bool,
        user_agent: Option<String>,
    ) -> Self {
        Self {
            request_id,
            api_key_id: api_key_id.map(str::to_string),
            channel_id: Some(last.channel_id.clone()),
            group_id: group_id.map(str::to_string),
            requested_model: model.to_string(),
            actual_model: Some(last.target_model.clone()),
            input_tokens: last.input_tokens,
            output_tokens: last.output_tokens,
            cache_read_tokens: last.cache_read,
            cache_creation_tokens: last.cache_creation,
            cost: last.cost,
            latency_ms: Some(last.latency_ms as i32),
            ttft_ms,
            status_code: Some(last.status_code as i32),
            error_message: last.error_message.clone(),
            endpoint_type: Some(last.upstream_endpoint.as_str().to_string()),
            request_type: if last.needs_conversion {
                "conversion".to_string()
            } else {
                "passthrough".to_string()
            },
            request_content,
            response_content,
            is_stream,
            upstream_key_hint: Some(last.upstream_key_hint.clone()),
            attempts: channel_attempts,
            user_agent,
        }
    }

    /// 构造选择阶段失败时的最小记录（503 + "请求未到达上游"）
    #[allow(clippy::too_many_arguments)]
    fn minimal_for_select_failure(
        request_id: Option<String>,
        api_key_id: Option<&str>,
        group_id: Option<&str>,
        model: &str,
        request_content: Option<String>,
        response_content: Option<String>,
        channel_attempts: Vec<crate::stats::recorder::ChannelAttempt>,
        is_stream: bool,
        user_agent: Option<String>,
        status_code: i32,
        error_message: &str,
    ) -> Self {
        Self {
            request_id,
            api_key_id: api_key_id.map(str::to_string),
            channel_id: None,
            group_id: group_id.map(str::to_string),
            requested_model: model.to_string(),
            actual_model: None,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            cost: None,
            latency_ms: None,
            ttft_ms: None,
            status_code: Some(status_code),
            error_message: Some(error_message.to_string()),
            endpoint_type: None,
            request_type: "unknown".to_string(),
            request_content,
            response_content,
            is_stream,
            upstream_key_hint: None,
            attempts: channel_attempts,
            user_agent,
        }
    }
}

/// 保存单条请求日志（汇总所有尝试）
#[allow(clippy::too_many_arguments)]
pub(super) async fn save_request_record(
    state: &ProxyState,
    request_id: Option<String>,
    api_key_id: Option<&str>,
    group_id: Option<&str>,
    model: &str,
    request_content: Option<String>,
    response_content: Option<String>,
    attempts: &[AttemptStats],
    ttft_ms: Option<i32>,
    is_stream: bool,
    user_agent: Option<String>,
    select_error: Option<&ProxyError>,
) {
    // 构造 attempts 快照（用于记录日志）
    let channel_attempts: Vec<crate::stats::recorder::ChannelAttempt> = attempts
        .iter()
        .map(|a| crate::stats::recorder::ChannelAttempt {
            channel_id: a.channel_id.clone(),
            channel_name: None,
            status: if (200..400).contains(&a.status_code) {
                "success".to_string()
            } else {
                "failed".to_string()
            },
            duration_ms: a.latency_ms,
            error: a.error_message.clone(),
            upstream_key_hint: Some(a.upstream_key_hint.clone()),
        })
        .collect();

    let record = match attempts.last() {
        Some(last) => crate::stats::recorder::RequestRecord::from_last_attempt(
            last,
            request_id,
            api_key_id,
            group_id,
            model,
            request_content,
            response_content,
            channel_attempts,
            ttft_ms,
            is_stream,
            user_agent,
        ),
        None => {
            let (status, msg) = select_error
                .map(|e| match e {
                    ProxyError::ModelNotFound(m) => (404, m.clone()),
                    ProxyError::NoAvailableChannel(m) => (503, m.clone()),
                    ProxyError::ModelNotSupported(m) => (400, m.clone()),
                    _ => (502, e.to_string()),
                })
                .unwrap_or((503, "请求未到达上游".to_string()));
            crate::stats::recorder::RequestRecord::minimal_for_select_failure(
                request_id,
                api_key_id,
                group_id,
                model,
                request_content,
                response_content,
                channel_attempts,
                is_stream,
                user_agent,
                status,
                &msg,
            )
        }
    };

    let _ = state.stats_recorder.record_request(record).await;

    // 速率限制：更新 TPM 计数
    if let Some(key_id) = api_key_id {
        let total_input: i32 = attempts.iter().map(|a| a.input_tokens).sum();
        let total_output: i32 = attempts.iter().map(|a| a.output_tokens).sum();
        if total_input > 0 || total_output > 0 {
            state
                .rate_limiter
                .record_tokens(key_id, total_input as u64, total_output as u64)
                .await;
        }
    }
}

/// 执行单次代理请求
#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_proxy_request(
    state: &ProxyState,
    _api_key_id: Option<&str>,
    upstream_api_key: &str,
    upstream_key_hint: &str,
    headers: &HeaderMap,
    body: &serde_json::Value,
    client_endpoint: &EndpointType,
    selection: &SelectionResult,
    attempts: &mut Vec<AttemptStats>,
) -> Result<ProxySuccess, ProxyError> {
    // 追踪活跃请求数
    let channel_id = selection.channel.id.clone();
    state
        .lb_state
        .ensure_channel_status(&channel_id, selection.channel.max_concurrency)
        .await;
    state.lb_state.increment_active(&channel_id).await;
    let _guard = ActiveRequestGuard {
        state: state.clone(),
        channel_id: channel_id.clone(),
    };

    let prepared =
        prepare_proxy_request(headers, body, client_endpoint, selection, upstream_api_key).await?;
    let start_time = std::time::Instant::now();

    let response = state
        .http_client
        .post(&prepared.url)
        .timeout(std::time::Duration::from_secs(
            selection.channel.timeout_secs,
        ))
        .headers(prepared.headers)
        .body(prepared.body)
        .send()
        .await
        .map_err(|e| ProxyError::RequestError(e.to_string()))?;

    let latency_ms = start_time.elapsed().as_millis() as i64;
    let status = response.status();
    let response_body = response.text().await.unwrap_or_default();

    let body_value: serde_json::Value = serde_json::from_str(&response_body).unwrap_or_default();
    let status_u16 = status.as_u16();

    let (input_tokens, output_tokens, cache_read, cache_creation) =
        if (200..400).contains(&status_u16) {
            let (i, o, cr, cc) = extract_usage(&body_value, &prepared.upstream_endpoint);
            if i == 0 && o == 0 {
                let req_text = extract_request_text(body);
                let resp_text = extract_response_text(&body_value);
                (
                    estimate_tokens(&req_text),
                    estimate_tokens(&resp_text),
                    cr,
                    cc,
                )
            } else {
                (i, o, cr, cc)
            }
        } else {
            (0, 0, 0, 0)
        };
    let cost = if input_tokens > 0 || output_tokens > 0 {
        Some(
            state
                .model_registry
                .calculate_cost(
                    &prepared.target_model,
                    input_tokens,
                    output_tokens,
                    cache_read,
                    cache_creation,
                )
                .await,
        )
    } else {
        None
    };

    attempts.push(AttemptStats {
        channel_id: prepared.channel_id.clone(),
        target_model: prepared.target_model.clone(),
        upstream_endpoint: prepared.upstream_endpoint.clone(),
        needs_conversion: prepared.needs_conversion,
        latency_ms,
        status_code: status_u16,
        input_tokens,
        output_tokens,
        cache_read,
        cache_creation,
        cost,
        error_message: if !status.is_success() {
            Some(response_body[..response_body.len().min(500)].to_string())
        } else {
            None
        },
        upstream_key_hint: upstream_key_hint.to_string(),
    });

    if !status.is_success() {
        tracing::warn!(
            "Upstream error: channel={}, status={}, body={}",
            prepared.channel_id,
            status,
            &response_body[..response_body.len().min(300)]
        );
        return Err(ProxyError::UpstreamError {
            status,
            body: response_body,
        });
    }

    state
        .lb_state
        .record_success(&prepared.channel_id, latency_ms as f64)
        .await;

    let final_body = if prepared.needs_conversion {
        let finalized = RelayPipeline::finalize_response_async(
            client_endpoint.clone(),
            prepared.upstream_endpoint.clone(),
            body_value,
            status.as_u16(),
        )
        .await
        .map_err(|e| ProxyError::TransformError(e.to_string()))?;
        serde_json::to_vec(&finalized.body).unwrap_or_default()
    } else {
        response_body.into_bytes()
    };

    Ok(ProxySuccess {
        status,
        body: final_body,
    })
}

/// 非流式代理请求（支持重试和排队）
pub async fn proxy_request(
    state: &ProxyState,
    request_id: &str,
    api_key_id: Option<&str>,
    headers: &HeaderMap,
    body: &serde_json::Value,
    client_endpoint: &EndpointType,
) -> Result<ProxySuccess, ProxyError> {
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
    let candidates = match super::build_relay_candidates(
        state,
        &model,
        client_endpoint,
        session_hash.as_deref(),
    )
    .await
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

        Ok(ProxySuccess {
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

/// 执行单次流式代理请求
#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_proxy_stream(
    state: &ProxyState,
    request_id: String,
    api_key_id: Option<&str>,
    upstream_api_key: &str,
    upstream_key_hint: String,
    group_id: Option<String>,
    headers: &HeaderMap,
    body: &serde_json::Value,
    client_endpoint: &EndpointType,
    selection: &SelectionResult,
    attempts: &mut Vec<AttemptStats>,
    _permit: Option<tokio::sync::OwnedSemaphorePermit>,
) -> Result<
    (
        StatusCode,
        std::pin::Pin<
            Box<
                dyn futures::Stream<Item = Result<Bytes, std::convert::Infallible>>
                    + Send
                    + 'static,
            >,
        >,
        String,
        Option<i32>,
    ),
    ProxyError,
> {
    // 追踪活跃请求数
    let channel_id = selection.channel.id.clone();
    state
        .lb_state
        .ensure_channel_status(&channel_id, selection.channel.max_concurrency)
        .await;
    state.lb_state.increment_active(&channel_id).await;
    // 流式请求的 guard 需要在 spawned task 中手动 decrement，因为流的生命周期超出本函数
    // 但如果本函数在发送前就返回 Err，需要确保 decrement
    let active_decremented = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let active_decremented_clone = active_decremented.clone();
    let state_for_decrement = state.clone();
    let channel_id_for_decrement = channel_id.clone();

    let prepared =
        prepare_proxy_request(headers, body, client_endpoint, selection, upstream_api_key).await?;
    let start_time = std::time::Instant::now();

    let response = state
        .http_client
        .post(&prepared.url)
        .timeout(std::time::Duration::from_secs(
            selection.channel.timeout_secs,
        ))
        .headers(prepared.headers)
        .body(prepared.body)
        .send()
        .await
        .map_err(|e| {
            decrement_active_once(
                &state_for_decrement,
                &channel_id_for_decrement,
                &active_decremented_clone,
            );
            ProxyError::RequestError(e.to_string())
        })?;

    if !response.status().is_success() {
        let latency_ms = start_time.elapsed().as_millis() as i64;
        let status = response.status();
        let response_body = response.text().await.unwrap_or_default();

        attempts.push(AttemptStats {
            channel_id: prepared.channel_id.clone(),
            target_model: prepared.target_model.clone(),
            upstream_endpoint: prepared.upstream_endpoint.clone(),
            needs_conversion: prepared.needs_conversion,
            latency_ms,
            status_code: status.as_u16(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read: 0,
            cache_creation: 0,
            cost: None,
            error_message: Some(response_body[..response_body.len().min(500)].to_string()),
            upstream_key_hint: upstream_key_hint.clone(),
        });

        decrement_active_once(
            &state_for_decrement,
            &channel_id_for_decrement,
            &active_decremented_clone,
        );
        return Err(ProxyError::UpstreamError {
            status,
            body: response_body,
        });
    }

    use futures::StreamExt;
    let mut upstream_stream = response.bytes_stream();

    let mut initial_buffer = Vec::new();
    while let Some(chunk) = upstream_stream.next().await {
        match chunk {
            Ok(bytes) => {
                initial_buffer.extend_from_slice(&bytes);
                if find_sse_boundary(&initial_buffer).is_some() || initial_buffer.len() >= 64 * 1024
                {
                    break;
                }
            }
            Err(e) => {
                decrement_active_once(
                    &state_for_decrement,
                    &channel_id_for_decrement,
                    &active_decremented_clone,
                );
                return Err(ProxyError::RequestError(e.to_string()));
            }
        }
    }

    if let Some(event_end) = find_sse_boundary(&initial_buffer)
        && let Ok(text) = std::str::from_utf8(&initial_buffer[..event_end])
        && let Some(error) = extract_error_from_sse(text, &prepared.upstream_endpoint)
    {
        let latency_ms = start_time.elapsed().as_millis() as i64;
        let sanitized_error = sanitize_upstream_error(&error);

        attempts.push(AttemptStats {
            channel_id: prepared.channel_id.clone(),
            target_model: prepared.target_model.clone(),
            upstream_endpoint: prepared.upstream_endpoint.clone(),
            needs_conversion: prepared.needs_conversion,
            latency_ms,
            status_code: StatusCode::BAD_GATEWAY.as_u16(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read: 0,
            cache_creation: 0,
            cost: None,
            error_message: Some(sanitized_error),
            upstream_key_hint: upstream_key_hint.clone(),
        });

        decrement_active_once(
            &state_for_decrement,
            &channel_id_for_decrement,
            &active_decremented_clone,
        );
        return Err(ProxyError::UpstreamError {
            status: StatusCode::BAD_GATEWAY,
            body: error,
        });
    }

    let upstream_stream = futures::stream::iter(
        (!initial_buffer.is_empty())
            .then(|| Ok::<Bytes, reqwest::Error>(Bytes::from(initial_buffer))),
    )
    .chain(upstream_stream);

    let state_clone = state.clone();
    let channel_id_clone = prepared.channel_id.clone();
    let model_clone = prepared.model.clone();
    let target_model_clone = prepared.target_model.clone();
    let upstream_endpoint_clone = prepared.upstream_endpoint.clone();
    let client_endpoint_clone = client_endpoint.clone();
    let needs_conversion = prepared.needs_conversion;
    let api_key_id_clone = api_key_id.map(|s| s.to_string());
    let request_content_clone = serde_json::to_string(&body).ok();
    let req_text_for_estimation = extract_request_text(body);
    let request_id_clone = request_id;

    // 提前 clone 给 spawn 任务使用
    let sc_channel_id = channel_id_clone.clone();
    let sc_extras = selection.channel.extras.clone();
    let sc_model = model_clone.clone();
    let sc_target_model = target_model_clone.clone();
    let sc_client_endpoint = client_endpoint_clone.clone();
    let sc_needs_conversion = needs_conversion;
    let sc_api_key_id = api_key_id_clone.clone();
    let sc_request_content = request_content_clone.clone();
    let sc_upstream_key_hint = upstream_key_hint.clone();
    let sc_user_agent = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let stats_recorder = state.stats_recorder.clone();
    let rate_limiter = state.rate_limiter.clone();
    let attempts_snapshot: Vec<crate::stats::recorder::ChannelAttempt> = attempts
        .iter()
        .map(|a| crate::stats::recorder::ChannelAttempt {
            channel_id: a.channel_id.clone(),
            channel_name: None,
            status: if (200..400).contains(&a.status_code) {
                "success".to_string()
            } else {
                "failed".to_string()
            },
            duration_ms: a.latency_ms,
            error: a.error_message.clone(),
            upstream_key_hint: Some(a.upstream_key_hint.clone()),
        })
        .collect();

    // === SSE 流式背压：使用有界 mpsc channel 替代 async_stream ===
    const STREAM_BUFFER_SIZE: usize = 16;
    let (stream_tx, stream_rx) =
        tokio::sync::mpsc::channel::<Result<Bytes, std::convert::Infallible>>(STREAM_BUFFER_SIZE);

    // 辅助：发送数据，客户端断开时返回 false
    async fn stream_send(
        tx: &tokio::sync::mpsc::Sender<Result<Bytes, std::convert::Infallible>>,
        data: Bytes,
    ) -> bool {
        tx.send(Ok(data)).await.is_ok()
    }

    // 流处理 + 统计记录全在一个 spawned task 里
    let active_decremented_spawn = active_decremented.clone();
    tokio::spawn(async move {
        // _permit 随任务生命周期存在，释放 semaphore 许可
        let _permit = _permit;
        let mut stream = std::pin::pin!(upstream_stream);
        let mut last_usage: Option<serde_json::Value> = None;
        let mut input_usage: Option<serde_json::Value> = None;
        let mut buffer = Vec::new();
        let mut collected_text = String::new();
        let mut collected_reasoning = String::new();
        let mut collected_tool_calls: Vec<serde_json::Value> = Vec::new();
        let mut stream_error: Option<String> = None;
        let mut ttft_ms: Option<i32> = None;
        let mut first_token_seen = false;

        // 思维链规范化器：仅在渠道 extras.thinking.{extract_tags,fix_signature} 启用时创建
        let extract_think_tags = thinking_flag(&sc_extras, "extract_tags");
        let fix_signature = thinking_flag(&sc_extras, "fix_signature");

        let mut passthrough_normalizer: Option<PassthroughNormalizer> =
            if extract_think_tags || fix_signature {
                Some(PassthroughNormalizer::new())
            } else {
                None
            };
        let mut conversion_extractor: Option<ThinkingTagExtractor> = if extract_think_tags {
            Some(ThinkingTagExtractor::new())
        } else {
            None
        };

        if needs_conversion {
            let mut converter = RelayPipeline::create_stream_converter(
                &client_endpoint_clone,
                &upstream_endpoint_clone,
            )
            .expect("stream converter creation should not fail for conversion path")
            .expect("conversion path should return Some converter");

            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        buffer.extend_from_slice(&bytes);

                        while let Some(event_end) = find_sse_boundary(&buffer) {
                            let event_bytes = buffer[..event_end].to_vec();
                            buffer = buffer[event_end..].to_vec();

                            if event_bytes.iter().all(|b| *b == b'\n' || *b == b'\r') {
                                continue;
                            }

                            if let Ok(text) = std::str::from_utf8(&event_bytes)
                                && let Some(source) =
                                    extract_usage_from_sse(text, &upstream_endpoint_clone)
                            {
                                apply_sse_usage(source, &mut last_usage, &mut input_usage);
                            }
                            let mut is_error_event = false;
                            if stream_error.is_none()
                                && let Ok(text) = std::str::from_utf8(&event_bytes)
                                && let Some(error) =
                                    extract_error_from_sse(text, &upstream_endpoint_clone)
                            {
                                stream_error = Some(error);
                                is_error_event = true;
                            }
                            if is_error_event {
                                if let Some(error) = stream_error.as_deref()
                                    && !stream_send(
                                        &stream_tx,
                                        Bytes::from(format_stream_error_event(
                                            error,
                                            &client_endpoint_clone,
                                        )),
                                    )
                                    .await
                                {
                                    break;
                                }
                                continue;
                            }

                            if !first_token_seen {
                                ttft_ms = Some(start_time.elapsed().as_millis() as i32);
                                first_token_seen = true;
                            }

                            match RelayPipeline::decode_stream_event(
                                &upstream_endpoint_clone,
                                &event_bytes,
                            ) {
                                Ok(Some(mut llm_stream)) => {
                                    // 思维链规范化先于 stats：让 stats 看到的是清理后的内容
                                    if let Some(ref mut extractor) = conversion_extractor {
                                        extractor.extract(&mut llm_stream);
                                    }
                                    // 收集内容用于统计
                                    if let Some(choice) = llm_stream.first_choice() {
                                        if let Some(crate::protocol::model::Content::Text(t)) =
                                            &choice.delta.content
                                            && !t.is_empty()
                                        {
                                            collected_text.push_str(t);
                                        }
                                        if let Some(r) = &choice.delta.reasoning_content {
                                            collected_reasoning.push_str(r);
                                        }
                                        if let Some(tcs) = &choice.delta.tool_calls {
                                            for tc in tcs {
                                                if !tc.id.is_empty() {
                                                    // 新 tool call
                                                    collected_tool_calls.push(serde_json::json!({
                                                        "id": tc.id,
                                                        "name": tc.function.name,
                                                        "arguments": tc.function.arguments,
                                                    }));
                                                } else if let Some(last) =
                                                    collected_tool_calls.last_mut()
                                                {
                                                    // 续传 chunk — 追加 arguments
                                                    if let Some(args) = last["arguments"].as_str() {
                                                        let combined = format!(
                                                            "{}{}",
                                                            args, tc.function.arguments
                                                        );
                                                        last["arguments"] =
                                                            serde_json::Value::String(combined);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    // 有状态转换：一个事件可能产生多个 SSE 输出
                                    match converter.convert(&llm_stream) {
                                        Ok(converted_events) => {
                                            for converted in converted_events {
                                                if !stream_send(&stream_tx, Bytes::from(converted))
                                                    .await
                                                {
                                                    break;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            tracing::error!("Stream conversion error: {}", e);
                                        }
                                    }
                                }
                                Ok(None) => {}
                                Err(e) => {
                                    tracing::error!("Stream outbound conversion error: {}", e);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Upstream stream error: {}", e);
                        break;
                    }
                }
            }

            if !buffer.is_empty() && !buffer.iter().all(|b| *b == b'\n' || *b == b'\r') {
                if let Ok(text) = std::str::from_utf8(&buffer)
                    && let Some(source) = extract_usage_from_sse(text, &upstream_endpoint_clone)
                {
                    apply_sse_usage(source, &mut last_usage, &mut input_usage);
                }
                let mut is_error_event = false;
                if stream_error.is_none()
                    && let Ok(text) = std::str::from_utf8(&buffer)
                    && let Some(error) = extract_error_from_sse(text, &upstream_endpoint_clone)
                {
                    stream_error = Some(error);
                    is_error_event = true;
                }
                if !is_error_event {
                    if let Ok(Some(llm_stream)) =
                        RelayPipeline::decode_stream_event(&upstream_endpoint_clone, &buffer)
                    {
                        match converter.convert(&llm_stream) {
                            Ok(converted_events) => {
                                for converted in converted_events {
                                    stream_send(&stream_tx, Bytes::from(converted)).await;
                                }
                            }
                            Err(e) => {
                                tracing::error!("Stream conversion error (drain): {}", e);
                            }
                        }
                    }
                } else if let Some(error) = stream_error.as_deref() {
                    stream_send(
                        &stream_tx,
                        Bytes::from(format_stream_error_event(error, &client_endpoint_clone)),
                    )
                    .await;
                }
            }

            // 发送流结束事件
            match converter.finish() {
                Ok(finish_events) => {
                    for event_bytes in finish_events {
                        stream_send(&stream_tx, Bytes::from(event_bytes)).await;
                    }
                }
                Err(e) => {
                    tracing::error!("Stream finish error: {}", e);
                }
            }
        } else {
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        buffer.extend_from_slice(&bytes);

                        let mut client_disconnected = false;
                        while let Some(event_end) = find_sse_boundary(&buffer) {
                            let event_bytes = buffer[..event_end].to_vec();
                            buffer = buffer[event_end..].to_vec();

                            if event_bytes.iter().all(|b| *b == b'\n' || *b == b'\r') {
                                continue;
                            }

                            // TTFT：在第一个有效 SSE 事件处记录（比"第一个 chunk"更精确）
                            if !first_token_seen {
                                ttft_ms = Some(start_time.elapsed().as_millis() as i32);
                                first_token_seen = true;
                            }

                            if let Ok(text) = std::str::from_utf8(&event_bytes) {
                                if let Some(source) =
                                    extract_usage_from_sse(text, &upstream_endpoint_clone)
                                {
                                    apply_sse_usage(source, &mut last_usage, &mut input_usage);
                                }
                                if stream_error.is_none()
                                    && let Some(error) =
                                        extract_error_from_sse(text, &upstream_endpoint_clone)
                                {
                                    stream_error = Some(error);
                                }
                                collect_sse_content(
                                    text,
                                    &upstream_endpoint_clone,
                                    &mut collected_text,
                                    &mut collected_reasoning,
                                    &mut collected_tool_calls,
                                );
                            }

                            // 直通路径：可选的 thinking 规范化
                            if let Some(ref mut normalizer) = passthrough_normalizer {
                                let events =
                                    normalizer.process_sse(&event_bytes, &upstream_endpoint_clone);
                                for evt in events {
                                    if !stream_send(&stream_tx, Bytes::from(evt)).await {
                                        client_disconnected = true;
                                        break;
                                    }
                                }
                            } else if !stream_send(&stream_tx, Bytes::from(event_bytes)).await {
                                client_disconnected = true;
                            }
                            if client_disconnected {
                                break;
                            }
                        }
                        if client_disconnected {
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::error!("Stream error: {}", e);
                        break;
                    }
                }
            }

            // 处理 buffer 中残余的最后一个事件
            if !buffer.is_empty()
                && !buffer.iter().all(|b| *b == b'\n' || *b == b'\r')
                && let Ok(text) = std::str::from_utf8(&buffer)
            {
                if let Some(source) = extract_usage_from_sse(text, &upstream_endpoint_clone) {
                    apply_sse_usage(source, &mut last_usage, &mut input_usage);
                }
                if stream_error.is_none()
                    && let Some(error) = extract_error_from_sse(text, &upstream_endpoint_clone)
                {
                    stream_error = Some(error);
                }
                collect_sse_content(
                    text,
                    &upstream_endpoint_clone,
                    &mut collected_text,
                    &mut collected_reasoning,
                    &mut collected_tool_calls,
                );
            }
        }

        // === 流结束：统计记录（即使客户端断开也会执行） ===
        drop(stream_tx); // 关闭 channel，通知 ReceiverStream 结束

        let latency_ms = start_time.elapsed().as_millis() as i64;
        let (mut input_tokens, mut output_tokens, cache_read, cache_creation) =
            match &upstream_endpoint_clone {
                EndpointType::Anthropic => {
                    let input = input_usage
                        .as_ref()
                        .and_then(|u| u["input_tokens"].as_i64())
                        .filter(|&v| v > 0)
                        .or_else(|| {
                            last_usage
                                .as_ref()
                                .and_then(|u| u["usage"]["input_tokens"].as_i64())
                        })
                        .unwrap_or(0) as i32;
                    let output = last_usage
                        .as_ref()
                        .and_then(|u| u["usage"]["output_tokens"].as_i64())
                        .unwrap_or(0) as i32;
                    let cache_read = input_usage
                        .as_ref()
                        .and_then(|u| u["cache_read_input_tokens"].as_i64())
                        .filter(|&v| v > 0)
                        .or_else(|| {
                            last_usage
                                .as_ref()
                                .and_then(|u| u["usage"]["cache_read_input_tokens"].as_i64())
                        })
                        .unwrap_or(0) as i32;
                    let cache_creation = input_usage
                        .as_ref()
                        .and_then(|u| u["cache_creation_input_tokens"].as_i64())
                        .filter(|&v| v > 0)
                        .or_else(|| {
                            last_usage
                                .as_ref()
                                .and_then(|u| u["usage"]["cache_creation_input_tokens"].as_i64())
                        })
                        .unwrap_or(0) as i32;
                    (input, output, cache_read, cache_creation)
                }
                _ => last_usage
                    .map(|u| extract_usage(&u, &upstream_endpoint_clone))
                    .unwrap_or((0, 0, 0, 0)),
            };

        // 兜底估算
        if input_tokens == 0 && output_tokens == 0 {
            input_tokens = estimate_tokens(&req_text_for_estimation);
            output_tokens = estimate_tokens(&collected_text);
        }

        let cost = if input_tokens > 0 || output_tokens > 0 {
            Some(
                state_clone
                    .model_registry
                    .calculate_cost(
                        &target_model_clone,
                        input_tokens,
                        output_tokens,
                        cache_read,
                        cache_creation,
                    )
                    .await,
            )
        } else {
            None
        };

        let (status_code, error_message, response_content) = if let Some(error) = stream_error {
            state_clone
                .lb_state
                .record_failure(&channel_id_clone, false)
                .await;
            (502i32, Some(sanitize_upstream_error(&error)), Some(error))
        } else {
            state_clone
                .lb_state
                .record_success_with_ttft(
                    &channel_id_clone,
                    latency_ms as f64,
                    ttft_ms.map(|v| v as f64),
                )
                .await;
            let resp = if collected_text.is_empty()
                && collected_reasoning.is_empty()
                && collected_tool_calls.is_empty()
                && input_tokens == 0
                && output_tokens == 0
            {
                None
            } else {
                let mut resp_json = serde_json::json!({
                    "content": collected_text,
                    "usage": {
                        "input_tokens": input_tokens,
                        "output_tokens": output_tokens,
                        "cache_read_tokens": cache_read,
                        "cache_creation_tokens": cache_creation,
                    }
                });
                if !collected_reasoning.is_empty() {
                    resp_json["reasoning"] = serde_json::json!(collected_reasoning);
                }
                if !collected_tool_calls.is_empty() {
                    resp_json["tool_calls"] = serde_json::json!(collected_tool_calls);
                }
                Some(resp_json.to_string())
            };
            (200i32, None, resp)
        };

        // 直接写入统计（不再需要 stats_tx/stats_rx）
        let mut channel_attempts = attempts_snapshot;
        channel_attempts.push(crate::stats::recorder::ChannelAttempt {
            channel_id: sc_channel_id.clone(),
            channel_name: None,
            status: if (200..400).contains(&status_code) {
                "success".into()
            } else {
                "failed".into()
            },
            duration_ms: latency_ms,
            error: error_message.clone(),
            upstream_key_hint: Some(sc_upstream_key_hint.clone()),
        });

        let rate_limit_key = sc_api_key_id.clone();
        let decrement_channel_id = sc_channel_id.clone();
        let record = crate::stats::recorder::RequestRecord {
            request_id: Some(request_id_clone),
            api_key_id: sc_api_key_id,
            channel_id: Some(sc_channel_id),
            group_id,
            requested_model: sc_model,
            actual_model: Some(sc_target_model),
            input_tokens,
            output_tokens,
            cache_read_tokens: cache_read,
            cache_creation_tokens: cache_creation,
            cost,
            latency_ms: Some(latency_ms as i32),
            ttft_ms,
            status_code: Some(status_code),
            error_message,
            endpoint_type: Some(sc_client_endpoint.as_str().to_string()),
            request_type: if sc_needs_conversion {
                "conversion".into()
            } else {
                "passthrough".into()
            },
            request_content: sc_request_content,
            response_content,
            is_stream: true,
            upstream_key_hint: Some(sc_upstream_key_hint),
            attempts: channel_attempts,
            user_agent: sc_user_agent,
        };

        match stats_recorder.record_request(record).await {
            Ok(()) => {
                if let Some(ref key_id) = rate_limit_key
                    && (input_tokens > 0 || output_tokens > 0)
                {
                    rate_limiter
                        .record_tokens(key_id, input_tokens as u64, output_tokens as u64)
                        .await;
                }
            }
            Err(e) => tracing::warn!("Failed to save stream stats: {}", e),
        }

        // 流结束，递减活跃请求数
        if !active_decremented_spawn.load(std::sync::atomic::Ordering::Relaxed) {
            active_decremented_spawn.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        state_clone
            .lb_state
            .decrement_active(&decrement_channel_id)
            .await;
    });

    // 将 mpsc Receiver 转换为 Stream 返回给 axum
    let response_stream = futures::stream::unfold(stream_rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    });

    Ok((
        StatusCode::OK,
        Box::pin(response_stream),
        "text/event-stream".to_string(),
        None,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::recorder::{ChannelAttempt, RequestRecord};

    fn sample_attempt(status_code: u16) -> AttemptStats {
        AttemptStats {
            channel_id: "ch-1".into(),
            target_model: "gpt-4o".into(),
            upstream_endpoint: EndpointType::OpenAiChat,
            needs_conversion: false,
            latency_ms: 123,
            status_code,
            input_tokens: 10,
            output_tokens: 20,
            cache_read: 3,
            cache_creation: 0,
            cost: Some(0.001),
            error_message: None,
            upstream_key_hint: "sk-abcde...mnop".into(),
        }
    }

    fn sample_attempts() -> Vec<ChannelAttempt> {
        vec![ChannelAttempt {
            channel_id: "ch-1".into(),
            channel_name: None,
            status: "failed".into(),
            duration_ms: 100,
            error: Some("upstream 500".into()),
            upstream_key_hint: Some("sk-abcde...mnop".into()),
        }]
    }

    #[test]
    fn from_last_attempt_propagates_all_fields() {
        let last = sample_attempt(200);
        let record = RequestRecord::from_last_attempt(
            &last,
            Some("req-1".into()),
            Some("key-1"),
            Some("grp-1"),
            "gpt-4o",
            Some("hi".into()),
            Some("hello".into()),
            sample_attempts(),
            Some(45),
            false,
            Some("ua/1.0".into()),
        );
        assert_eq!(record.api_key_id.as_deref(), Some("key-1"));
        assert_eq!(record.channel_id.as_deref(), Some("ch-1"));
        assert_eq!(record.group_id.as_deref(), Some("grp-1"));
        assert_eq!(record.requested_model, "gpt-4o");
        assert_eq!(record.actual_model.as_deref(), Some("gpt-4o"));
        assert_eq!(record.input_tokens, 10);
        assert_eq!(record.output_tokens, 20);
        assert_eq!(record.cache_read_tokens, 3);
        assert_eq!(record.cost, Some(0.001));
        assert_eq!(record.latency_ms, Some(123));
        assert_eq!(record.ttft_ms, Some(45));
        assert_eq!(record.status_code, Some(200));
        assert_eq!(record.error_message, None);
        assert_eq!(record.endpoint_type.as_deref(), Some("openai_chat"));
        assert_eq!(record.request_type, "passthrough");
        assert!(!record.is_stream);
        assert_eq!(record.upstream_key_hint.as_deref(), Some("sk-abcde...mnop"));
        assert_eq!(record.attempts.len(), 1);
        assert_eq!(record.user_agent.as_deref(), Some("ua/1.0"));
    }

    #[test]
    fn from_last_attempt_marks_conversion_path() {
        let mut last = sample_attempt(200);
        last.needs_conversion = true;
        let record = RequestRecord::from_last_attempt(
            &last,
            None,
            None,
            None,
            "claude-sonnet",
            None,
            None,
            vec![],
            None,
            false,
            None,
        );
        assert_eq!(record.request_type, "conversion");
    }

    #[test]
    fn from_last_attempt_marks_passthrough_path() {
        let last = sample_attempt(200);
        let record = RequestRecord::from_last_attempt(
            &last,
            None,
            None,
            None,
            "gpt-4o",
            None,
            None,
            vec![],
            None,
            true,
            None,
        );
        assert_eq!(record.request_type, "passthrough");
        assert!(record.is_stream);
    }

    #[test]
    fn from_last_attempt_preserves_error_message_on_failed_status() {
        let mut last = sample_attempt(503);
        last.error_message = Some("upstream timeout".into());
        let record = RequestRecord::from_last_attempt(
            &last,
            None,
            None,
            None,
            "gpt-4o",
            None,
            None,
            vec![],
            None,
            false,
            None,
        );
        assert_eq!(record.status_code, Some(503));
        assert_eq!(record.error_message.as_deref(), Some("upstream timeout"));
    }

    #[test]
    fn minimal_for_select_failure_fills_503_and_marker_text() {
        let record = RequestRecord::minimal_for_select_failure(
            None,
            Some("key-1"),
            Some("grp-1"),
            "gpt-4o",
            Some("hi".into()),
            None,
            sample_attempts(),
            false,
            Some("ua/1.0".into()),
            503,
            "请求未到达上游",
        );
        assert_eq!(record.status_code, Some(503));
        assert_eq!(record.error_message.as_deref(), Some("请求未到达上游"));
        assert!(record.channel_id.is_none());
        assert!(record.actual_model.is_none());
        assert_eq!(record.input_tokens, 0);
        assert_eq!(record.output_tokens, 0);
        assert!(record.cost.is_none());
        assert!(record.latency_ms.is_none());
        assert!(record.ttft_ms.is_none());
        assert!(record.endpoint_type.is_none());
        assert!(record.upstream_key_hint.is_none());
        assert_eq!(record.request_type, "unknown");
        assert_eq!(record.attempts.len(), 1);
    }

    // ============================================================
    // M2-S1: AttemptStats → AttemptTrace adapter characterization
    // ============================================================

    /// 验证成功的 AttemptStats 映射到 AttemptTrace 时字段正确
    #[test]
    fn attempt_trace_log_adapter_success_maps_fields_and_serializes() {
        use crate::scheduler::trace::{AttemptStatus, AttemptTrace};

        let stats = AttemptStats {
            channel_id: "ch-1".into(),
            target_model: "gpt-4o".into(),
            upstream_endpoint: EndpointType::OpenAiChat,
            needs_conversion: false,
            latency_ms: 150,
            status_code: 200,
            input_tokens: 10,
            output_tokens: 5,
            cache_read: 0,
            cache_creation: 0,
            cost: Some(0.001),
            error_message: None,
            upstream_key_hint: "sk-abc...xyz".into(),
        };

        let trace: AttemptTrace = stats.to_trace(1, "gpt-4o", &EndpointType::OpenAiChat);

        assert_eq!(trace.attempt_no, 1);
        assert_eq!(trace.status, AttemptStatus::Success);
        assert!(trace.reason.is_none());
        assert_eq!(trace.channel_id.as_deref(), Some("ch-1"));
        assert_eq!(trace.upstream_model.as_deref(), Some("gpt-4o"));
        assert_eq!(trace.upstream_endpoint.as_deref(), Some("openai_chat"));
        assert_eq!(trace.duration_ms, Some(150));
        assert_eq!(trace.upstream_key_hint.as_deref(), Some("sk-abc...xyz"));

        // JSON 序列化验证 snake_case 和关键字段
        let json = serde_json::to_string(&trace).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["attempt_no"], 1);
        assert_eq!(parsed["status"], "success");
        assert_eq!(parsed["reason"], serde_json::Value::Null);
        assert_eq!(parsed["upstream_endpoint"], "openai_chat");
    }

    /// 验证失败的 AttemptStats 映射到 AttemptTrace 时 reason 和 status 正确
    #[test]
    fn attempt_trace_log_adapter_failure_maps_reason_and_status() {
        use crate::scheduler::trace::{AttemptStatus, AttemptTrace};

        let mut stats = sample_attempt(500);
        stats.error_message = Some("upstream timeout".into());

        let trace: AttemptTrace = stats.to_trace(3, "gpt-4o", &EndpointType::OpenAiChat);

        assert_eq!(trace.attempt_no, 3);
        assert_eq!(trace.status, AttemptStatus::Failed);
        assert_eq!(trace.reason.as_deref(), Some("upstream timeout"));

        let json = serde_json::to_string(&trace).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["status"], "failed");
        assert_eq!(parsed["reason"], "upstream timeout");
    }

    // ============================================================
    // 端到端：mock 本地 upstream + 真实 ProxyState 调 proxy_request
    // ============================================================

    use crate::db::Database;
    use crate::proxy::ProxyState;
    use crate::stats::model::ModelRegistry;
    use axum::{Router, routing::post};

    async fn spawn_mock_upstream() -> String {
        use axum::extract::Json as AxJson;

        async fn mock_chat(AxJson(_body): AxJson<serde_json::Value>) -> AxJson<serde_json::Value> {
            AxJson(serde_json::json!({
                "id": "chatcmpl-test",
                "object": "chat.completion",
                "model": "gpt-4o",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "hi back"},
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 10,
                    "completion_tokens": 5,
                    "total_tokens": 15
                }
            }))
        }

        let app = Router::new().route("/v1/chat/completions", post(mock_chat));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        // 等服务器就绪：发起一次 OPTIONS 失败会得到 405，证明已监听
        let url = format!("http://{}/v1/chat/completions", addr);
        for _ in 0..20 {
            if reqwest::Client::new()
                .request(reqwest::Method::POST, &url)
                .body("{}")
                .send()
                .await
                .is_ok()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        url
    }

    async fn make_state_with_channel(upstream_url: &str) -> (ProxyState, sqlx::SqlitePool) {
        let db_path = format!("/tmp/galaxy_execute_{}.db", uuid::Uuid::now_v7());
        let _ = std::fs::remove_file(&db_path);
        let db_url = format!("sqlite:{}?mode=rwc", db_path);
        let db = Database::new(&db_url).await.unwrap();
        let pool = db.pool().clone();

        // base_url 是 URL 去掉 /chat/completions 后的部分
        let base_url = upstream_url.trim_end_matches("/chat/completions");
        let channel_id = "ch-mock";
        let api_keys = r#"[{"key":"sk-mock","note":"","enabled":true}]"#;
        let endpoints = format!(
            r#"[{{"type":"openai_chat","base_url":"{}","enabled":true}}]"#,
            base_url
        );
        let models = r#"["gpt-4o"]"#;
        sqlx::query(
            "INSERT INTO channels (id, name, api_keys, endpoints, models, enabled) \
             VALUES (?, 'mock', ?, ?, ?, 1)",
        )
        .bind(channel_id)
        .bind(api_keys)
        .bind(&endpoints)
        .bind(models)
        .execute(&pool)
        .await
        .unwrap();

        let group_id = "grp-mock";
        sqlx::query("INSERT INTO groups (id, name, enabled) VALUES (?, 'gpt-4o', 1)")
            .bind(group_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO group_items (id, group_id, channel_id, model_name) \
             VALUES ('item-mock', ?, ?, 'gpt-4o')",
        )
        .bind(group_id)
        .bind(channel_id)
        .execute(&pool)
        .await
        .unwrap();

        let registry = ModelRegistry::new(pool.clone());
        let state = ProxyState::new(pool.clone(), registry).await;
        (state, pool)
    }

    #[tokio::test]
    async fn proxy_request_passes_through_to_local_upstream() {
        let upstream = spawn_mock_upstream().await;
        let (state, pool) = make_state_with_channel(&upstream).await;

        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}]
        });
        let headers = axum::http::HeaderMap::new();
        let result = proxy_request(
            &state,
            "test-request-id",
            Some("key-1"),
            &headers,
            &body,
            &EndpointType::OpenAiChat,
        )
        .await
        .expect("proxy should succeed");

        assert_eq!(result.status, 200);
        let resp: serde_json::Value = serde_json::from_slice(&result.body).unwrap();
        assert_eq!(resp["choices"][0]["message"]["content"], "hi back");
        assert_eq!(resp["usage"]["prompt_tokens"], 10);

        // 日志应被记录
        let count: i32 = sqlx::query_scalar("SELECT COUNT(*) FROM usage_logs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1, "应记录 1 条请求日志");
    }

    #[tokio::test]
    async fn proxy_request_no_available_channel_returns_error() {
        // 没有 channel/group 时
        let db_path = format!("/tmp/galaxy_execute_empty_{}.db", uuid::Uuid::now_v7());
        let _ = std::fs::remove_file(&db_path);
        let db_url = format!("sqlite:{}?mode=rwc", db_path);
        let db = Database::new(&db_url).await.unwrap();
        let pool = db.pool().clone();
        let registry = ModelRegistry::new(pool.clone());
        let state = ProxyState::new(pool.clone(), registry).await;

        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}]
        });
        let headers = axum::http::HeaderMap::new();
        let result = proxy_request(
            &state,
            "test-request-id",
            Some("key-1"),
            &headers,
            &body,
            &EndpointType::OpenAiChat,
        )
        .await;
        let err = match result {
            Ok(_) => panic!("expected NoAvailableChannel, got Ok"),
            Err(e) => e,
        };
        assert!(matches!(err, ProxyError::ModelNotFound(_)), "got {:?}", err);

        // 失败也应记录日志（minimal_for_select_failure 路径）
        let count: i32 = sqlx::query_scalar("SELECT COUNT(*) FROM usage_logs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1, "渠道选择失败也应记录日志");
        let status: Option<i32> = sqlx::query_scalar("SELECT status_code FROM usage_logs LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(status, Some(404));
    }
}
