use async_trait::async_trait;
use axum::body::Bytes;
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::domain::channel::EndpointType;
use crate::service::stats::attempt::AttemptStats;
use crate::service::stats::recorder::{
    channel_attempts_snapshot, record_stream_completion, RequestRecord,
};
use crate::service::stats::usage::{calculate_cost, resolve_stream_usage};
use crate::llm::protocol::sse::{
    extract_error_from_sse, find_sse_boundary, sanitize_upstream_error,
};
use crate::llm::relay::prepare::{extract_request_text, failed_attempt_stats, prepare_proxy_request};
use crate::llm::relay::run::{
    RelayAttemptError, RelayCandidate, RelayRequest, RelayStreamAttemptExecutor,
    RelayStreamAttemptResult,
};
use crate::error::proxy::{ProxyError, sse_stream_error_status};
use crate::app_state::AppState;
use crate::llm::plugin::PluginContext;
use crate::llm::scheduler::selector::SelectionResult;
use axum::http::StatusCode;
use futures::Stream;

use super::stream_error::{decrement_active_once, StreamPanicGuard};
use super::stream_key_loop::run_key_stream_loop;

pub(crate) type RelayBodyStream = Pin<Box<dyn Stream<Item = Result<Bytes, Infallible>> + Send>>;

/// 流式代理执行器：将 RelayStreamRun 的候选迭代与真实 SSE 执行连接。
#[derive(Clone)]
pub(crate) struct ProxyStreamRelayExecutor {
    pub(super) state: AppState,
    request_id: String,
    headers: axum::http::HeaderMap,
    body: serde_json::Value,
    client_endpoint: EndpointType,
    api_key_id: Option<String>,
    pub(super) queue_permit: Arc<Mutex<Option<tokio::sync::OwnedSemaphorePermit>>>,
    pub(super) attempt_stats: Arc<Mutex<Vec<AttemptStats>>>,
}

impl ProxyStreamRelayExecutor {
    pub(crate) fn new(
        state: AppState,
        request_id: String,
        headers: axum::http::HeaderMap,
        body: serde_json::Value,
        client_endpoint: EndpointType,
        api_key_id: Option<String>,
        queue_permit: Option<tokio::sync::OwnedSemaphorePermit>,
    ) -> Self {
        Self {
            state,
            request_id,
            headers,
            body,
            client_endpoint,
            api_key_id,
            queue_permit: Arc::new(Mutex::new(queue_permit)),
            attempt_stats: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn take_attempt_stats(&self) -> Vec<AttemptStats> {
        let mut stats = self
            .attempt_stats
            .lock()
            .expect("attempt_stats mutex poisoned");
        std::mem::take(&mut *stats)
    }

    async fn build_selection(
        &self,
        candidate: &RelayCandidate,
    ) -> Result<SelectionResult, RelayAttemptError> {
        let channel = self
            .state
            .get_channel(&candidate.channel_id)
            .await
            .map_err(|e| RelayAttemptError::new(503, e.to_string()))?;

        let endpoint = channel
            .find_best_endpoint(&self.client_endpoint)
            .ok_or_else(|| {
                RelayAttemptError::new(
                    503,
                    format!(
                        "channel {} has no available endpoint",
                        candidate.channel_id,
                    ),
                )
            })?;

        Ok(SelectionResult {
            channel,
            target_model: candidate.target_model.clone(),
            endpoint,
            route_id: candidate.route_id.clone(),
        })
    }

    /// 执行单次流式代理请求（从 proxy/execute.rs 内迁）
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn execute_proxy_stream(
        &self,
        upstream_api_key: &str,
        upstream_key_hint: &str,
        selection: &SelectionResult,
        attempts: &mut Vec<AttemptStats>,
        permit: Option<tokio::sync::OwnedSemaphorePermit>,
    ) -> Result<(StatusCode, RelayBodyStream, String, Option<i32>), ProxyError> {
        let channel_id = selection.channel.id.clone();
        self.state
            .lb_state
            .ensure_channel_status(
                &channel_id,
                selection.channel.max_concurrency,
                selection.channel.failure_threshold,
                selection.channel.blacklist_minutes,
            )
            .await;
        self.state.lb_state.increment_active(&channel_id).await;
        let active_decremented = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let active_decremented_clone = active_decremented.clone();
        let state_for_decrement = self.state.clone();
        let channel_id_for_decrement = channel_id.clone();

        let prepared = prepare_proxy_request(
            &self.headers,
            &self.body,
            &self.client_endpoint,
            selection,
            upstream_api_key,
            &self.state.plugin_chain,
        )
        .await?;
        let start_time = std::time::Instant::now();

        let response = self
            .state
            .proxy_http_client
            .post(&prepared.url)
            .timeout(std::time::Duration::from_secs(
                selection.channel.timeout_secs,
            ))
            .headers(prepared.headers)
            .body(prepared.body)
            .send()
            .await;
        let response = match response {
            Ok(r) => r,
            Err(e) => {
                // 网络层失败也要记录 attempt，否则 usage_logs 会丢这条请求的 channel/latency
                attempts.push(failed_attempt_stats(
                    &self.body,
                    &prepared.channel_id,
                    &prepared.target_model,
                    &prepared.upstream_endpoint,
                    prepared.needs_conversion,
                    start_time.elapsed().as_millis() as i64,
                    502,
                    e.to_string(),
                    upstream_key_hint,
                ));
                decrement_active_once(
                    &state_for_decrement,
                    &channel_id_for_decrement,
                    &active_decremented_clone,
                );
                return Err(ProxyError::RequestError(e.to_string()));
            }
        };

        if !response.status().is_success() {
            let latency_ms = start_time.elapsed().as_millis() as i64;
            let status = response.status();
            let response_body = response.text().await.unwrap_or_default();

            attempts.push(failed_attempt_stats(
                &self.body,
                &prepared.channel_id,
                &prepared.target_model,
                &prepared.upstream_endpoint,
                prepared.needs_conversion,
                latency_ms,
                status.as_u16(),
                response_body[..response_body.len().min(500)].to_string(),
                upstream_key_hint,
            ));

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
                    if find_sse_boundary(&initial_buffer).is_some()
                        || initial_buffer.len() >= 64 * 1024
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
            let sse_status = sse_stream_error_status(&error);

            attempts.push(failed_attempt_stats(
                &self.body,
                &prepared.channel_id,
                &prepared.target_model,
                &prepared.upstream_endpoint,
                prepared.needs_conversion,
                latency_ms,
                sse_status.as_u16(),
                sanitized_error,
                upstream_key_hint,
            ));

            decrement_active_once(
                &state_for_decrement,
                &channel_id_for_decrement,
                &active_decremented_clone,
            );
            return Err(ProxyError::UpstreamError {
                status: sse_status,
                body: error,
            });
        }

        let upstream_stream = futures::stream::iter(
            (!initial_buffer.is_empty())
                .then(|| Ok::<Bytes, reqwest::Error>(Bytes::from(initial_buffer))),
        )
        .chain(upstream_stream);

        let state_clone = self.state.clone();
        let channel_id_clone = prepared.channel_id.clone();
        let model_clone = prepared.model.clone();
        let target_model_clone = prepared.target_model.clone();
        let upstream_endpoint_clone = prepared.upstream_endpoint.clone();
        let client_endpoint_clone = self.client_endpoint.clone();
        let needs_conversion = prepared.needs_conversion;
        let api_key_id_clone = self.api_key_id.clone();
        let request_content_clone = serde_json::to_string(&self.body).ok();
        let req_text_for_estimation = extract_request_text(&self.body);
        let request_id_clone = self.request_id.clone();

        let sc_channel_id = channel_id_clone.clone();
        let sc_model = model_clone.clone();
        let sc_target_model = target_model_clone.clone();
        let sc_client_endpoint = client_endpoint_clone.clone();
        let sc_needs_conversion = needs_conversion;
        let sc_api_key_id = api_key_id_clone.clone();
        let sc_request_content = request_content_clone.clone();
        let sc_upstream_key_hint = upstream_key_hint.to_string();
        let sc_user_agent = self
            .headers
            .get("user-agent")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let stats_recorder = self.state.stats_recorder.clone();
        let rate_limiter = self.state.rate_limiter.clone();
        let attempts_snapshot = channel_attempts_snapshot(attempts);

        // === SSE 流式背压：使用有界 mpsc channel 替代 async_stream ===
        const STREAM_BUFFER_SIZE: usize = 16;
        let (stream_tx, stream_rx) = tokio::sync::mpsc::channel::<
            Result<Bytes, std::convert::Infallible>,
        >(STREAM_BUFFER_SIZE);

        // 流处理 + 统计记录全在一个 spawned task 里
        let active_decremented_spawn = active_decremented.clone();
        let route_id = selection.route_id.clone();

        // 为 spawn task 的 panic 兜底预先构造 RequestRecord（关键字段的快照）。
        // 即使 spawn 内部 panic，也能由 StreamPanicGuard 的 Drop 写一条简化失败日志，
        // 保证请求不漏记（"CC 已失败但 usage_logs 缺失"的核心修复之一）。
        let sc_endpoint_for_guard = sc_client_endpoint.as_str().to_string();
        let panic_guard = StreamPanicGuard::new(
            state_clone.clone(),
            RequestRecord {
                request_id: Some(request_id_clone.clone()),
                api_key_id: sc_api_key_id.clone(),
                channel_id: Some(sc_channel_id.clone()),
                route_id: route_id.clone(),
                requested_model: sc_model.clone(),
                actual_model: Some(sc_target_model.clone()),
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                cost: None,
                latency_ms: None,
                ttft_ms: None,
                status_code: None,
                error_message: None,
                endpoint_type: Some(sc_endpoint_for_guard),
                request_type: if sc_needs_conversion {
                    "conversion".to_string()
                } else {
                    "passthrough".to_string()
                },
                request_content: sc_request_content.clone(),
                response_content: None,
                is_stream: true,
                upstream_key_hint: Some(sc_upstream_key_hint.clone()),
                attempts: vec![],
                user_agent: sc_user_agent.clone(),
            },
        );

        tokio::spawn(async move {
            // permit 随任务生命周期存在，释放 semaphore 许可
            let _permit = permit;
            let mut _panic_guard = panic_guard;
            let mut stream = std::pin::pin!(upstream_stream);

            // thinking 流式 processor：needs_conversion 路径累积 reasoning 承接落库
            // （passthrough 路径仍用 collect_sse_content）
            let plugin_ctx = PluginContext {
                upstream_endpoint: upstream_endpoint_clone.clone(),
                channel_id: sc_channel_id.clone(),
                host_key: sc_upstream_key_hint.clone(),
                client_name: sc_user_agent.clone(),
            };
            let thinking_processor = state_clone
                .plugin_chain
                .new_stream_processor(&plugin_ctx)
                .await;

            // 流消费累积状态（两循环 + 收尾共用）与只读上下文
            let mut st = super::stream_loop::StreamCollectState {
                thinking_processor,
                ..Default::default()
            };
            let loop_ctx = super::stream_loop::LoopCtx {
                upstream_endpoint: &upstream_endpoint_clone,
                client_endpoint: &sc_client_endpoint,
                start_time: &start_time,
            };

            if needs_conversion {
                super::stream_loop::run_conversion_loop(
                    &mut st,
                    &loop_ctx,
                    stream.as_mut(),
                    &stream_tx,
                )
                .await;
            } else {
                super::stream_loop::run_passthrough_loop(
                    &mut st,
                    &loop_ctx,
                    stream.as_mut(),
                    &stream_tx,
                )
                .await;
            }

            // === 流结束：统计记录（即使客户端断开也会执行） ===
            drop(stream_tx); // 关闭 channel，通知 ReceiverStream 结束

            let latency_ms = start_time.elapsed().as_millis() as i64;
            let usage = resolve_stream_usage(
                &upstream_endpoint_clone,
                st.last_usage,
                st.input_usage,
                &req_text_for_estimation,
                &st.collected_text,
            );
            let cost =
                calculate_cost(&state_clone.model_registry, &target_model_clone, usage).await;
            let input_tokens = usage.input_tokens;
            let output_tokens = usage.output_tokens;
            let cache_read = usage.cache_read;
            let cache_creation = usage.cache_creation;

            // thinking 流式 hook：流结束取累积 reasoning，合并进 collected_reasoning
            // （needs_conversion 路径删了内联收集，由 processor 承接；passthrough 仍由 collect_sse_content 填）
            if let Some(p) = st.thinking_processor.as_mut()
                && let Some(r) = p.finish_reasoning()
            {
                st.collected_reasoning.push_str(&r);
            }

            let (status_code, error_message, response_content) = if let Some(error) = st.stream_error.take() {
                state_clone
                    .lb_state
                    .record_failure(&channel_id_clone, false)
                    .await;
                (sse_stream_error_status(&error).as_u16() as i32, Some(sanitize_upstream_error(&error)), Some(error))
            } else {
                state_clone
                    .lb_state
                    .record_success_with_ttft(
                        &channel_id_clone,
                        latency_ms as f64,
                        st.ttft_ms.map(|v| v as f64),
                    )
                    .await;
                // per-key 熔断：流成功结束，重置该 key 的熔断状态
                state_clone
                    .lb_state
                    .circuit_breaker
                    .record_success(&channel_id_clone, &sc_upstream_key_hint)
                    .await;
                let resp = if st.collected_text.is_empty()
                    && st.collected_reasoning.is_empty()
                    && st.collected_tool_calls.is_empty()
                    && input_tokens == 0
                    && output_tokens == 0
                {
                    None
                } else {
                    let mut resp_json = serde_json::json!({
                        "content": st.collected_text,
                        "usage": {
                            "input_tokens": input_tokens,
                            "output_tokens": output_tokens,
                            "cache_read_tokens": cache_read,
                            "cache_creation_tokens": cache_creation,
                        }
                    });
                    if !st.collected_reasoning.is_empty() {
                        resp_json["reasoning"] = serde_json::json!(st.collected_reasoning);
                    }
                    if !st.collected_tool_calls.is_empty() {
                        resp_json["tool_calls"] = serde_json::json!(st.collected_tool_calls);
                    }
                    Some(resp_json.to_string())
                };
                (200i32, None, resp)
            };

            // 直接写入统计
            let decrement_channel_id = sc_channel_id.clone();
            record_stream_completion(
                stats_recorder,
                rate_limiter,
                request_id_clone,
                sc_api_key_id,
                sc_channel_id,
                route_id,
                sc_model,
                sc_target_model,
                input_tokens,
                output_tokens,
                cache_read,
                cache_creation,
                cost,
                latency_ms,
                st.ttft_ms,
                status_code,
                error_message,
                sc_client_endpoint.as_str().to_string(),
                sc_needs_conversion,
                sc_request_content,
                response_content,
                sc_upstream_key_hint,
                attempts_snapshot,
                sc_user_agent,
            )
            .await;

            // 正常路径已经写了完整日志，解除 panic 兜底 guard
            _panic_guard.disarm();

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
}

#[async_trait]
impl RelayStreamAttemptExecutor for ProxyStreamRelayExecutor {
    async fn is_channel_available(&self, channel_id: &str) -> bool {
        self.state.lb_state.is_channel_available(channel_id).await
    }

    async fn on_attempt_failed(&self, channel_id: &str, error: &ProxyError) {
        // key-retryable 错误（401/402/429/503/余额不足等）已经走 per-key 熔断器，
        // 不应计入渠道级黑名单，否则上游瞬时过载（如 mimo 503）会把整个渠道连带健康的端点一起拉黑。
        if error.is_key_retryable() {
            return;
        }
        let is_server_error = matches!(error.classify(), crate::error::proxy::ErrorClass::UpstreamRetryable);
        self.state
            .lb_state
            .record_failure(channel_id, is_server_error)
            .await;
    }

    async fn execute_stream(
        &self,
        _request: &RelayRequest,
        candidate: &RelayCandidate,
    ) -> RelayStreamAttemptResult {
        let selection = match self.build_selection(candidate).await {
            Ok(s) => s,
            Err(e) => {
                // build_selection 失败时也记录 AttemptStats，保证 channel_id/request_type 可观测
                if let Ok(mut stats) = self.attempt_stats.lock() {
                    stats.push(AttemptStats {
                        channel_id: candidate.channel_id.clone(),
                        target_model: candidate.target_model.clone(),
                        upstream_endpoint: EndpointType::OpenAiChat,
                        needs_conversion: false,
                        latency_ms: 0,
                        status_code: e.status_code,
                        input_tokens: 0,
                        output_tokens: 0,
                        cache_read: 0,
                        cache_creation: 0,
                        cost: None,
                        error_message: Some(e.message.clone()),
                        upstream_key_hint: String::new(),
                    });
                }
                return RelayStreamAttemptResult {
                    response: Err(e),
                    response_written: false,
                };
            }
        };

        // 503 同渠道退避重试（参考 sub2api HandleSelectionExhausted 模式）
        let first_pass = run_key_stream_loop(self, &selection, candidate).await;
        if first_pass.should_retry_503() {
            tracing::warn!(
                "所有 key 返回 503，2s 后同渠道重试 (stream): channel={}",
                candidate.channel_id
            );
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            self.state
                .lb_state
                .circuit_breaker
                .reset_channel(&candidate.channel_id)
                .await;
            return run_key_stream_loop(self, &selection, candidate)
                .await
                .into_stream_result();
        }
        first_pass.into_stream_result()
    }
}

#[cfg(test)]
mod tests {
    use crate::infra::db::Database;
    use crate::llm::plugin::PluginChain;
    use crate::service::pricing::model::ModelRegistry;
    use crate::llm::relay::pipeline::proxy_stream;
    use axum::body::Body;
    use axum::{Router, routing::post};
    use futures::StreamExt;

    use super::*;

    /// mock upstream 返回 OpenAI Chat SSE 流（含 reasoning_content delta）。
    async fn spawn_mock_upstream_sse() -> String {
        async fn mock_stream() -> axum::response::Response {
            let body = concat!(
                "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"think1\"},\"finish_reason\":null}]}\n\n",
                "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"think2\"},\"finish_reason\":null}]}\n\n",
                "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"answer\"},\"finish_reason\":null}]}\n\n",
                "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"gpt-4o\",\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15}}\n\n",
                "data: [DONE]\n\n",
            );
            axum::response::Response::builder()
                .header("content-type", "text/event-stream")
                .body(Body::from(body))
                .unwrap()
        }
        let app = Router::new().route("/v1/chat/completions", post(mock_stream));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
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

    async fn make_state_with_channel(upstream_url: &str) -> (AppState, sqlx::SqlitePool) {
        let db_path = format!("/tmp/galaxy_stream_{}.db", uuid::Uuid::now_v7());
        let _ = std::fs::remove_file(&db_path);
        let db_url = format!("sqlite:{}?mode=rwc", db_path);
        let db = Database::new(&db_url).await.unwrap();
        let pool = db.pool().clone();

        let base_url = upstream_url.trim_end_matches("/chat/completions");
        let api_keys = r#"[{"key":"sk-mock","note":"","enabled":true}]"#;
        let endpoints = format!(
            r#"[{{"type":"openai_chat","base_url":"{}","enabled":true}}]"#,
            base_url
        );
        sqlx::query(
            "INSERT INTO channels (id, name, api_keys, endpoints, models, enabled) \
             VALUES ('ch-mock', 'mock', ?, ?, '[\"gpt-4o\"]', 1)",
        )
        .bind(api_keys)
        .bind(&endpoints)
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query("INSERT INTO routes (id, name, enabled) VALUES ('grp-mock', 'gpt-4o', 1)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO route_items (id, route_id, channel_id, model_name) \
             VALUES ('item-mock', 'grp-mock', 'ch-mock', 'gpt-4o')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let registry = ModelRegistry::new(pool.clone());
        let mut state = AppState::new_for_test(pool.clone(), registry);
        // 启用 thinking 插件（conversion 路径 reasoning 由 processor 承接）
        state.plugin_chain = PluginChain::build_default_chain();
        state
            .plugin_chain
            .refresh(&*state.repositories.settings)
            .await
            .unwrap();
        (state, pool)
    }

    /// 等待 spawn task 异步落库完成，返回 response_content。
    async fn wait_for_response_content(pool: &sqlx::SqlitePool) -> Option<String> {
        for _ in 0..40 {
            let row: Option<(Option<String>,)> =
                sqlx::query_as("SELECT response_content FROM usage_payloads LIMIT 1")
                    .fetch_optional(pool)
                    .await
                    .unwrap();
            if let Some((Some(r),)) = row {
                return Some(r);
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        None
    }

    /// conversion 路径（client Anthropic ↔ upstream OpenAiChat）：thinking processor
    /// 累积 reasoning_content，流结束喂 resp_json["reasoning"] 落库（承接原内联收集）。
    #[tokio::test]
    async fn proxy_stream_conversion_collects_reasoning_via_processor() {
        let upstream = spawn_mock_upstream_sse().await;
        let (state, pool) = make_state_with_channel(&upstream).await;

        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 100,
            "stream": true
        });
        let headers = axum::http::HeaderMap::new();
        let (status, stream, _ct) = proxy_stream(
            &state,
            "test-stream-conv",
            Some("key-1"),
            &headers,
            &body,
            &EndpointType::Anthropic,
        )
        .await
        .expect("proxy_stream should succeed");
        assert_eq!(status, 200);

        // 消费完整转发流（流结束 spawn task 才落库）
        let _collected: Vec<_> = stream.collect().await;

        let resp = wait_for_response_content(&pool)
            .await
            .expect("usage_logs 应落库且 response_content 填充");
        // thinking processor 累积 think1+think2 → resp_json["reasoning"]
        assert!(
            resp.contains("think1think2"),
            "response_content 应含累积 reasoning: {}",
            resp
        );
    }

    /// mock upstream 返回 OpenAI Chat SSE 流（content 混入 `<think>`，模拟 DeepSeek/QwQ）。
    async fn spawn_mock_upstream_sse_think_in_content() -> String {
        async fn mock_stream() -> axum::response::Response {
            let body = concat!(
                "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"<think>plan</think>final\"},\"finish_reason\":null}]}\n\n",
                "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" answer\"},\"finish_reason\":null}]}\n\n",
                "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"gpt-4o\",\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15}}\n\n",
                "data: [DONE]\n\n",
            );
            axum::response::Response::builder()
                .header("content-type", "text/event-stream")
                .body(Body::from(body))
                .unwrap()
        }
        let app = Router::new().route("/v1/chat/completions", post(mock_stream));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
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

    /// passthrough（client==upstream==OpenAiChat）+ thinking：正文 content 混入的 `<think>`
    /// 应被剥离归入 reasoning_content 转发（decode→observe→reencode 路径）。
    #[tokio::test]
    async fn proxy_stream_passthrough_strips_think_for_openai_chat() {
        let upstream = spawn_mock_upstream_sse_think_in_content().await;
        let (state, _pool) = make_state_with_channel(&upstream).await;

        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true
        });
        let headers = axum::http::HeaderMap::new();
        let (status, stream, _ct) = proxy_stream(
            &state,
            "test-pt",
            Some("key-1"),
            &headers,
            &body,
            &EndpointType::OpenAiChat, // client==upstream → passthrough
        )
        .await
        .expect("proxy_stream should succeed");
        assert_eq!(status, 200);

        // 合并转发的 SSE，解析 content / reasoning_content
        let mut content = String::new();
        let mut reasoning = String::new();
        let items: Vec<_> = stream.collect().await;
        for item in items {
            let bytes = item.unwrap(); // Item=Result<Bytes, Infallible>，Err 不可能
            let Ok(text) = std::str::from_utf8(&bytes[..]) else {
                continue;
            };
            for line in text.lines() {
                let Some(data) = line.strip_prefix("data: ") else {
                    continue;
                };
                if data == "[DONE]" {
                    continue;
                }
                let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else {
                    continue;
                };
                if let Some(c) = v["choices"][0]["delta"]["content"].as_str() {
                    content.push_str(c);
                }
                if let Some(r) = v["choices"][0]["delta"]["reasoning_content"].as_str() {
                    reasoning.push_str(r);
                }
            }
        }
        // `<think>plan</think>` 剥离：正文 "final answer"，reasoning "plan"
        assert!(
            !content.contains("<think>"),
            "正文不应含 <think>: {}",
            content
        );
        assert!(
            !content.contains("plan"),
            "plan 应剥离到 reasoning，正文不应含: {}",
            content
        );
        assert_eq!(content, "final answer");
        assert_eq!(reasoning, "plan");
    }

    /// mock upstream 返回 OpenAI Chat SSE 流：content 混 `<think>`（触发 thinking rewrite）+
    /// 一个 tool_calls delta。`with_index` 控制 tool_calls 项是否带 OpenAI 标准 `index`。
    async fn spawn_mock_upstream_sse_tool_calls(with_index: bool) -> String {
        let tc = if with_index {
            r#"{"index":0,"id":"call_1","type":"function","function":{"name":"search","arguments":"{\"q\":\"rust\"}"}}"#
        } else {
            r#"{"id":"call_1","type":"function","function":{"name":"search","arguments":"{\"q\":\"rust\"}"}}"#
        };
        let body = format!(
            "data: {{\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"gpt-4o\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"<think>plan</think>final\"}},\"finish_reason\":null}}]}}\n\ndata: {{\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"gpt-4o\",\"choices\":[{{\"index\":0,\"delta\":{{\"tool_calls\":[{tc}]}},\"finish_reason\":null}}]}}\n\ndata: {{\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"gpt-4o\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n"
        );
        let body = std::sync::Arc::new(body);
        let app = Router::new().route("/v1/chat/completions", post({
            let body = body.clone();
            move || {
                let body = body.clone();
                async move {
                    axum::response::Response::builder()
                        .header("content-type", "text/event-stream")
                        .body(Body::from(body.as_bytes().to_vec()))
                        .unwrap()
                }
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
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

    /// passthrough + thinking rewrite 下，tool_calls delta 经 decode→observe→reencode 后
    /// index 必须保留/补全（gclm-agent 等 OpenAI 客户端 index 必填），且 reencode 后
    /// delta.role 不应是 default 的 "user"。
    async fn run_tool_call_index_case(with_index: bool) {
        let upstream = spawn_mock_upstream_sse_tool_calls(with_index).await;
        let (state, _pool) = make_state_with_channel(&upstream).await;

        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true
        });
        let headers = axum::http::HeaderMap::new();
        let (status, stream, _ct) = proxy_stream(
            &state,
            "test-tc",
            Some("key-1"),
            &headers,
            &body,
            &EndpointType::OpenAiChat, // client==upstream → passthrough + thinking rewrite
        )
        .await
        .expect("proxy_stream should succeed");
        assert_eq!(status, 200);

        let mut found_index = None;
        let mut found_role_user = false;
        let items: Vec<_> = stream.collect().await;
        for item in items {
            let bytes = item.unwrap();
            let Ok(text) = std::str::from_utf8(&bytes[..]) else {
                continue;
            };
            for line in text.lines() {
                let Some(data) = line.strip_prefix("data: ") else {
                    continue;
                };
                if data == "[DONE]" {
                    continue;
                }
                let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else {
                    continue;
                };
                if v["choices"][0]["delta"]["role"].as_str() == Some("user") {
                    found_role_user = true;
                }
                if let Some(arr) = v["choices"][0]["delta"]["tool_calls"].as_array()
                    && let Some(idx) = arr[0].get("index").and_then(|i| i.as_u64())
                {
                    found_index = Some(idx);
                }
            }
        }
        assert_eq!(
            found_index,
            Some(0),
            "tool_calls delta 必须带 index（OpenAI 流式协议 + gclm-agent index 必填）"
        );
        assert!(
            !found_role_user,
            "thinking rewrite reencode 后 delta.role 不应是 default 的 user"
        );
    }

    #[tokio::test]
    async fn proxy_stream_passthrough_tool_calls_keeps_index() {
        // 上游带 index：reencode 后保留
        run_tool_call_index_case(true).await;
    }

    #[tokio::test]
    async fn proxy_stream_passthrough_tool_calls_defaults_index_when_missing() {
        // 上游（智谱 GLM 等）tool_calls delta 不带 index：serde default 0，
        // gclm-agent 的 RawToolCallDelta.index 必填仍能满足。
        run_tool_call_index_case(false).await;
    }
}
