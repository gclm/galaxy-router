use async_trait::async_trait;
use axum::body::Bytes;
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::api::handlers::admin::channels::EndpointType;
use crate::metrics::attempt::AttemptStats;
use crate::metrics::recorder::{channel_attempts_snapshot, record_stream_completion};
use crate::metrics::usage::{calculate_cost, resolve_stream_usage};
use crate::protocol::sse::{
    apply_sse_usage, collect_sse_content, extract_error_from_sse, extract_usage_from_sse,
    find_sse_boundary, format_stream_error_event, sanitize_upstream_error,
};
use crate::protocol::thinking_normalizer::{PassthroughNormalizer, ThinkingTagExtractor};
use crate::relay::converter::RelayPipeline;
use crate::relay::prepare::{extract_request_text, prepare_proxy_request};
use crate::relay::run::{
    RelayAttemptError, RelayCandidate, RelayRequest, RelayStreamAttemptExecutor,
    RelayStreamAttemptResult, RelayStreamSuccess,
};
use crate::error::proxy::ProxyError;
use crate::relay::state::ProxyState;
use crate::scheduler::selector::SelectionResult;
use axum::http::StatusCode;
use futures::Stream;

pub(crate) type RelayBodyStream = Pin<Box<dyn Stream<Item = Result<Bytes, Infallible>> + Send>>;

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

/// 从渠道 extras JSON Map 读取 `extras.thinking.<key>` 布尔开关
fn thinking_flag(extras: &Option<serde_json::Map<String, serde_json::Value>>, key: &str) -> bool {
    extras
        .as_ref()
        .and_then(|e| e.get("thinking"))
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// 流式代理执行器：将 RelayStreamRun 的候选迭代与真实 SSE 执行连接。
#[derive(Clone)]
pub(crate) struct ProxyStreamRelayExecutor {
    state: ProxyState,
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
        state: ProxyState,
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
            group_id: candidate.group_id.clone(),
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
            .ensure_channel_status(&channel_id, selection.channel.max_concurrency)
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
        )
        .await?;
        let start_time = std::time::Instant::now();

        let response = self
            .state
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
                upstream_key_hint: upstream_key_hint.to_string(),
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
                upstream_key_hint: upstream_key_hint.to_string(),
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
        let sc_extras = selection.channel.extras.clone();
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
        let group_id = selection.group_id.clone();
        tokio::spawn(async move {
            // permit 随任务生命周期存在，释放 semaphore 许可
            let _permit = permit;
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
                                    let events = normalizer
                                        .process_sse(&event_bytes, &upstream_endpoint_clone);
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
                group_id,
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

    async fn on_attempt_failed(&self, channel_id: &str, _status_code: u16, is_server_error: bool) {
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
                return RelayStreamAttemptResult {
                    response: Err(e),
                    response_written: false,
                };
            }
        };

        let api_key_attempts = self.state.api_key_attempts(&selection.channel);
        let mut last_error = None;

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
                    &selection,
                    &mut local_attempts,
                    queue_permit,
                )
                .await;

            if let Ok(mut stats) = self.attempt_stats.lock() {
                *stats = local_attempts;
            }

            match result {
                Ok((status, stream, content_type, _ttft)) => {
                    return RelayStreamAttemptResult {
                        response: Ok(RelayStreamSuccess {
                            status,
                            stream,
                            content_type,
                            _capacity_permit: None,
                        }),
                        response_written: true,
                    };
                }
                Err(ProxyError::UpstreamError { status, body }) => {
                    let error = RelayAttemptError::new(status.as_u16(), sanitize_error_body(&body));
                    let proxy_error = ProxyError::UpstreamError {
                        status,
                        body: body.clone(),
                    };
                    if proxy_error.is_key_retryable() {
                        // per-key 熔断：记录该 key 失败，连续失败会熔断此 key（不影响其他 key）
                        self.state
                            .lb_state
                            .circuit_breaker
                            .record_failure(&candidate.channel_id, &key_hint)
                            .await;
                        last_error = Some(error);
                        continue;
                    }
                    return RelayStreamAttemptResult {
                        response: Err(error),
                        response_written: false,
                    };
                }
                Err(e) => {
                    return RelayStreamAttemptResult {
                        response: Err(RelayAttemptError::new(502, e.to_string())),
                        response_written: false,
                    };
                }
            }
        }

        RelayStreamAttemptResult {
            response: Err(
                last_error.unwrap_or_else(|| RelayAttemptError::new(500, "all api keys exhausted"))
            ),
            response_written: false,
        }
    }
}

fn sanitize_error_body(body: &str) -> String {
    body[..body.len().min(500)].to_string()
}
