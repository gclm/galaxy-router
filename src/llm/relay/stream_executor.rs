use async_trait::async_trait;
use axum::body::Bytes;
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::api::handlers::admin::channels::EndpointType;
use crate::service::stats::attempt::AttemptStats;
use crate::service::stats::recorder::{
    channel_attempts_snapshot, record_stream_completion, RequestRecord,
};
use crate::service::stats::usage::{calculate_cost, resolve_stream_usage};
use crate::protocol::sse::{
    apply_sse_usage, collect_sse_content, extract_error_from_sse, extract_usage_from_sse,
    find_sse_boundary, format_stream_error_event, sanitize_upstream_error,
};
use crate::relay::converter::RelayPipeline;
use crate::relay::prepare::{extract_request_text, failed_attempt_stats, prepare_proxy_request};
use crate::relay::run::{
    RelayAttemptError, RelayCandidate, RelayRequest, RelayStreamAttemptExecutor,
    RelayStreamAttemptResult, RelayStreamSuccess,
};
use crate::error::proxy::{ProxyError, sse_stream_error_status};
use crate::app_state::AppState;
use crate::llm::plugin::PluginContext;
use crate::scheduler::selector::SelectionResult;
use axum::http::StatusCode;
use futures::Stream;

use super::stream_error::{decrement_active_once, rewrite_thinking_passthrough, StreamPanicGuard};

pub(crate) type RelayBodyStream = Pin<Box<dyn Stream<Item = Result<Bytes, Infallible>> + Send>>;

/// 流式代理执行器：将 RelayStreamRun 的候选迭代与真实 SSE 执行连接。
#[derive(Clone)]
pub(crate) struct ProxyStreamRelayExecutor {
    state: AppState,
    request_id: String,
    headers: axum::http::HeaderMap,
    body: serde_json::Value,
    client_endpoint: EndpointType,
    api_key_id: Option<String>,
    queue_permit: Arc<Mutex<Option<tokio::sync::OwnedSemaphorePermit>>>,
    attempt_stats: Arc<Mutex<Vec<AttemptStats>>>,
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
    async fn execute_proxy_stream(
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

        // 辅助：发送数据，客户端断开时返回 false
        async fn stream_send(
            tx: &tokio::sync::mpsc::Sender<Result<Bytes, std::convert::Infallible>>,
            data: Bytes,
        ) -> bool {
            tx.send(Ok(data)).await.is_ok()
        }

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
            let mut last_usage: Option<serde_json::Value> = None;
            let mut input_usage: Option<serde_json::Value> = None;
            let mut buffer = Vec::new();
            let mut collected_text = String::new();
            let mut collected_reasoning = String::new();
            let mut collected_tool_calls: Vec<serde_json::Value> = Vec::new();
            let mut stream_error: Option<String> = None;
            let mut ttft_ms: Option<i32> = None;
            let mut first_token_seen = false;

            // thinking 流式 processor：needs_conversion 路径累积 reasoning 承接落库
            // （删原内联收集后由 processor 承接；passthrough 路径仍用 collect_sse_content）
            let ctx = PluginContext {
                upstream_endpoint: upstream_endpoint_clone.clone(),
                channel_id: sc_channel_id.clone(),
                host_key: sc_upstream_key_hint.clone(),
                client_name: sc_user_agent.clone(),
            };
            let mut thinking_processor = state_clone
                .plugin_chain
                .new_stream_processor(&ctx)
                .await;

            if needs_conversion {
                let mut converter = RelayPipeline::create_stream_converter(
                    &sc_client_endpoint,
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
                                                &sc_client_endpoint,
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
                                        // 收集内容用于统计
                                        if let Some(choice) = llm_stream.first_choice() {
                                            if let Some(crate::protocol::model::Content::Text(t)) =
                                                &choice.delta.content
                                                && !t.is_empty()
                                            {
                                                collected_text.push_str(t);
                                            }
                                            if let Some(tcs) = &choice.delta.tool_calls {
                                                for tc in tcs {
                                                    if !tc.id.is_empty() {
                                                        // 新 tool call
                                                        collected_tool_calls.push(
                                                            serde_json::json!({
                                                                "id": tc.id,
                                                                "name": tc.function.name,
                                                                "arguments": tc.function.arguments,
                                                            }),
                                                        );
                                                    } else if let Some(last) =
                                                        collected_tool_calls.last_mut()
                                                    {
                                                        // 续传 chunk — 追加 arguments
                                                        if let Some(args) =
                                                            last["arguments"].as_str()
                                                        {
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
                                        // thinking 流式 hook：剥离正文 content 的 <think> 归 reasoning_content（改写转发流）
                                        if let Some(p) = thinking_processor.as_mut() {
                                            p.observe(&mut llm_stream);
                                        }
                                        // 有状态转换：一个事件可能产生多个 SSE 输出
                                        match converter.convert(&llm_stream) {
                                            Ok(converted_events) => {
                                                for converted in converted_events {
                                                    if !stream_send(
                                                        &stream_tx,
                                                        Bytes::from(converted),
                                                    )
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
                            Bytes::from(format_stream_error_event(error, &sc_client_endpoint)),
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
                // thinking passthrough 改写：仅 OpenAiChat（DeepSeek/QwQ 直连客户端，<think> 混 content）。
                // 其他协议 passthrough 透传（其上游用结构化 thinking_delta，且 inbound encode 会破坏流结构）。
                let thinking_pt_rewrite = thinking_processor.is_some()
                    && upstream_endpoint_clone == EndpointType::OpenAiChat;
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
                                    // 落库收集：thinking 改写时 reasoning 由 processor 累积，collect 用 dummy 避免重复
                                    if thinking_pt_rewrite {
                                        let mut _dummy_reasoning = String::new();
                                        collect_sse_content(
                                            text,
                                            &upstream_endpoint_clone,
                                            &mut collected_text,
                                            &mut _dummy_reasoning,
                                            &mut collected_tool_calls,
                                        );
                                    } else {
                                        collect_sse_content(
                                            text,
                                            &upstream_endpoint_clone,
                                            &mut collected_text,
                                            &mut collected_reasoning,
                                            &mut collected_tool_calls,
                                        );
                                    }
                                }

                                // 转发：thinking 改写时 decode→observe→reencode；否则透传原始字节
                                let send_bytes = if thinking_pt_rewrite {
                                    rewrite_thinking_passthrough(
                                        &upstream_endpoint_clone,
                                        &event_bytes,
                                        thinking_processor.as_mut(),
                                    )
                                } else {
                                    Bytes::from(event_bytes)
                                };
                                if !stream_send(&stream_tx, send_bytes).await {
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
                    // 残余事件：thinking 改写时 reasoning 由 processor，collect 用 dummy（残余通常无 content/reasoning）
                    if thinking_pt_rewrite {
                        let mut _dummy_reasoning = String::new();
                        collect_sse_content(
                            text,
                            &upstream_endpoint_clone,
                            &mut collected_text,
                            &mut _dummy_reasoning,
                            &mut collected_tool_calls,
                        );
                    } else {
                        collect_sse_content(
                            text,
                            &upstream_endpoint_clone,
                            &mut collected_text,
                            &mut collected_reasoning,
                            &mut collected_tool_calls,
                        );
                    }
                }
            }

            // === 流结束：统计记录（即使客户端断开也会执行） ===
            drop(stream_tx); // 关闭 channel，通知 ReceiverStream 结束

            let latency_ms = start_time.elapsed().as_millis() as i64;
            let usage = resolve_stream_usage(
                &upstream_endpoint_clone,
                last_usage,
                input_usage,
                &req_text_for_estimation,
                &collected_text,
            );
            let cost =
                calculate_cost(&state_clone.model_registry, &target_model_clone, usage).await;
            let input_tokens = usage.input_tokens;
            let output_tokens = usage.output_tokens;
            let cache_read = usage.cache_read;
            let cache_creation = usage.cache_creation;

            // thinking 流式 hook：流结束取累积 reasoning，合并进 collected_reasoning
            // （needs_conversion 路径删了内联收集，由 processor 承接；passthrough 仍由 collect_sse_content 填）
            if let Some(p) = thinking_processor.as_mut()
                && let Some(r) = p.finish_reasoning()
            {
                collected_reasoning.push_str(&r);
            }

            let (status_code, error_message, response_content) = if let Some(error) = stream_error {
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
                        ttft_ms.map(|v| v as f64),
                    )
                    .await;
                // per-key 熔断：流成功结束，重置该 key 的熔断状态
                state_clone
                    .lb_state
                    .circuit_breaker
                    .record_success(&channel_id_clone, &sc_upstream_key_hint)
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
                ttft_ms,
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
        let first_pass = self.run_key_stream_loop(&selection, candidate).await;
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
            return self
                .run_key_stream_loop(&selection, candidate)
                .await
                .into_stream_result();
        }
        first_pass.into_stream_result()
    }
}

impl ProxyStreamRelayExecutor {
    /// 单轮遍历所有 key 执行流式代理请求。
    ///
    /// 返回 StreamKeyLoopOutcome 让上层判断是否要做"503 同渠道退避重试"。
    async fn run_key_stream_loop(
        &self,
        selection: &SelectionResult,
        candidate: &RelayCandidate,
    ) -> StreamKeyLoopOutcome {
        let api_key_attempts = self.state.api_key_attempts(&selection.channel);
        let mut last_error = None;
        let mut executed_count = 0u32;
        let mut all_executed_503 = true;

        for upstream_api_key in &api_key_attempts {
            let key_hint = selection.channel.key_hint(upstream_api_key);

            // per-key 熔断：该 key 已熔断则跳过，直接试下一个 key
            let (tripped, _) = self
                .state
                .lb_state
                .circuit_breaker
                .is_tripped(&candidate.channel_id, &key_hint)
                .await;
            if tripped {
                tracing::debug!(
                    "key 熔断跳过: channel={}, key={}",
                    candidate.channel_id,
                    key_hint
                );
                continue;
            }

            let mut local_attempts = self
                .attempt_stats
                .lock()
                .map(|mut stats| std::mem::take(&mut *stats))
                .unwrap_or_default();
            let queue_permit = self
                .queue_permit
                .lock()
                .expect("queue_permit mutex poisoned")
                .take();

            let result = self
                .execute_proxy_stream(
                    upstream_api_key,
                    &key_hint,
                    selection,
                    &mut local_attempts,
                    queue_permit,
                )
                .await;

            if let Ok(mut stats) = self.attempt_stats.lock() {
                *stats = local_attempts;
            }

            match result {
                Ok((status, stream, content_type, _ttft)) => {
                    return StreamKeyLoopOutcome::Success(RelayStreamSuccess {
                        status,
                        stream,
                        content_type,
                        _capacity_permit: None,
                    });
                }
                Err(ProxyError::UpstreamError { status, body }) => {
                    executed_count += 1;
                    let upstream_error = ProxyError::UpstreamError {
                        status,
                        body: body.clone(),
                    };
                    let error = RelayAttemptError::from_proxy_error(upstream_error);
                    let is_key_retryable = error
                        .proxy_error
                        .as_ref()
                        .map(|e| e.is_key_retryable())
                        .unwrap_or(false);

                    if status != axum::http::StatusCode::SERVICE_UNAVAILABLE {
                        all_executed_503 = false;
                    }

                    if is_key_retryable {
                        self.state
                            .lb_state
                            .circuit_breaker
                            .record_failure(&candidate.channel_id, &key_hint)
                            .await;
                        last_error = Some(error);
                        continue;
                    }
                    return StreamKeyLoopOutcome::NonKeyRetryableError(error);
                }
                Err(e) => {
                    // 仅 UpstreamError 走 key-retryable 路径（上方分支）。
                    // 其余 ProxyError 变体（DatabaseError / RequestError / TransformError 等）
                    // 非上游错误，不应触发换 key，直接 NonKeyRetryableError。
                    return StreamKeyLoopOutcome::NonKeyRetryableError(
                        RelayAttemptError::from_proxy_error(e),
                    );
                }
            }
        }

        StreamKeyLoopOutcome::AllKeysTried {
            last_error,
            all_executed_503: executed_count > 0 && all_executed_503,
        }
    }
}

/// 流式 run_key_stream_loop 的返回结果。
enum StreamKeyLoopOutcome {
    Success(RelayStreamSuccess),
    NonKeyRetryableError(RelayAttemptError),
    AllKeysTried {
        last_error: Option<RelayAttemptError>,
        all_executed_503: bool,
    },
}

impl StreamKeyLoopOutcome {
    fn should_retry_503(&self) -> bool {
        matches!(
            self,
            StreamKeyLoopOutcome::AllKeysTried {
                all_executed_503: true,
                ..
            }
        )
    }

    fn into_stream_result(self) -> RelayStreamAttemptResult {
        match self {
            StreamKeyLoopOutcome::Success(success) => RelayStreamAttemptResult {
                response: Ok(success),
                response_written: true,
            },
            StreamKeyLoopOutcome::NonKeyRetryableError(err)
            | StreamKeyLoopOutcome::AllKeysTried {
                last_error: Some(err),
                ..
            } => RelayStreamAttemptResult {
                response: Err(err),
                response_written: false,
            },
            StreamKeyLoopOutcome::AllKeysTried {
                last_error: None, ..
            } => RelayStreamAttemptResult {
                response: Err(RelayAttemptError::new(500, "all api keys exhausted")),
                response_written: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::infra::db::Database;
    use crate::llm::plugin::PluginChain;
    use crate::service::pricing::model::ModelRegistry;
    use crate::relay::pipeline::proxy_stream;
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
}
