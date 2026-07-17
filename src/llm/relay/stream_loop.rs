//! 从 stream_executor.rs 拆出（行为不变）：SSE 流消费循环（conversion / passthrough）。
//!
//! 两条路径共享 [`StreamCollectState`]（可变累积状态）与 [`LoopCtx`]（只读上下文），
//! 由 `execute_proxy_stream` 的 spawn 块构造后按 `needs_conversion` 选择一条调用。
//! 统计收尾仍在 spawn 块内联（读取本结构体字段）。

use std::convert::Infallible;

use axum::body::Bytes;
use futures::StreamExt;
use tokio::sync::mpsc::Sender;

use crate::domain::channel::EndpointType;
use crate::llm::plugin::StreamResponseProcessor;
use crate::llm::protocol::sse::{
    apply_sse_usage, collect_sse_content, extract_error_from_sse, extract_usage_from_sse,
    find_sse_boundary, format_stream_error_event,
};

use super::converter::RelayPipeline;
use super::stream_error::rewrite_thinking_passthrough;

/// 流消费过程中的可变累积状态（conversion 与 passthrough 两循环 + 收尾共用）。
#[derive(Default)]
pub(super) struct StreamCollectState {
    pub buffer: Vec<u8>,
    pub collected_text: String,
    pub collected_reasoning: String,
    pub collected_tool_calls: Vec<serde_json::Value>,
    pub last_usage: Option<serde_json::Value>,
    pub input_usage: Option<serde_json::Value>,
    pub stream_error: Option<String>,
    pub ttft_ms: Option<i32>,
    pub first_token_seen: bool,
    pub thinking_processor: Option<Box<dyn StreamResponseProcessor>>,
}

/// 两循环共享的只读上下文。
pub(super) struct LoopCtx<'a> {
    pub upstream_endpoint: &'a EndpointType,
    pub client_endpoint: &'a EndpointType,
    pub start_time: &'a std::time::Instant,
}

/// 发送一帧到客户端 mpsc；客户端断开返回 false。
async fn stream_send(tx: &Sender<Result<Bytes, Infallible>>, data: Bytes) -> bool {
    tx.send(Ok(data)).await.is_ok()
}

/// needs_conversion 路径：decode → observe → convert → send。
pub(super) async fn run_conversion_loop<S>(
    state: &mut StreamCollectState,
    ctx: &LoopCtx<'_>,
    mut stream: std::pin::Pin<&mut S>,
    stream_tx: &Sender<Result<Bytes, Infallible>>,
) where
    S: futures::Stream<Item = Result<Bytes, reqwest::Error>>,
{
    let mut converter = RelayPipeline::create_stream_converter(ctx.client_endpoint, ctx.upstream_endpoint)
        .expect("stream converter creation should not fail for conversion path")
        .expect("conversion path should return Some converter");

    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(bytes) => {
                state.buffer.extend_from_slice(&bytes);

                while let Some(event_end) = find_sse_boundary(&state.buffer) {
                    let event_bytes = state.buffer[..event_end].to_vec();
                    state.buffer = state.buffer[event_end..].to_vec();

                    if event_bytes.iter().all(|b| *b == b'\n' || *b == b'\r') {
                        continue;
                    }

                    if let Ok(text) = std::str::from_utf8(&event_bytes)
                        && let Some(source) = extract_usage_from_sse(text, ctx.upstream_endpoint)
                    {
                        apply_sse_usage(source, &mut state.last_usage, &mut state.input_usage);
                    }
                    let mut is_error_event = false;
                    if state.stream_error.is_none()
                        && let Ok(text) = std::str::from_utf8(&event_bytes)
                        && let Some(error) = extract_error_from_sse(text, ctx.upstream_endpoint)
                    {
                        state.stream_error = Some(error);
                        is_error_event = true;
                    }
                    if is_error_event {
                        if let Some(error) = state.stream_error.as_deref()
                            && !stream_send(
                                stream_tx,
                                Bytes::from(format_stream_error_event(error, ctx.client_endpoint)),
                            )
                            .await
                        {
                            break;
                        }
                        continue;
                    }

                    if !state.first_token_seen {
                        state.ttft_ms = Some(ctx.start_time.elapsed().as_millis() as i32);
                        state.first_token_seen = true;
                    }

                    match RelayPipeline::decode_stream_event(ctx.upstream_endpoint, &event_bytes) {
                        Ok(Some(mut llm_stream)) => {
                            // 收集内容用于统计
                            if let Some(choice) = llm_stream.first_choice() {
                                if let Some(crate::llm::protocol::model::Content::Text(t)) =
                                    &choice.delta.content
                                    && !t.is_empty()
                                {
                                    state.collected_text.push_str(t);
                                }
                                if let Some(tcs) = &choice.delta.tool_calls {
                                    for tc in tcs {
                                        let id = tc.id.as_deref().unwrap_or("");
                                        let name = tc
                                            .function
                                            .as_ref()
                                            .and_then(|f| f.name.as_deref())
                                            .unwrap_or("");
                                        let args = tc
                                            .function
                                            .as_ref()
                                            .and_then(|f| f.arguments.as_deref())
                                            .unwrap_or("");
                                        if !id.is_empty() {
                                            // 新 tool call
                                            state.collected_tool_calls.push(serde_json::json!({
                                                "id": id,
                                                "name": name,
                                                "arguments": args,
                                            }));
                                        } else if let Some(last) = state.collected_tool_calls.last_mut() {
                                            // 续传 chunk — 追加 arguments
                                            if let Some(prev) = last["arguments"].as_str() {
                                                let combined = format!("{}{}", prev, args);
                                                last["arguments"] =
                                                    serde_json::Value::String(combined);
                                            }
                                        }
                                    }
                                }
                            }
                            // thinking 流式 hook：剥离正文 content 的 <think> 归 reasoning_content（改写转发流）
                            if let Some(p) = state.thinking_processor.as_mut() {
                                p.observe(&mut llm_stream);
                            }
                            // 有状态转换：一个事件可能产生多个 SSE 输出
                            match converter.convert(&llm_stream) {
                                Ok(converted_events) => {
                                    for converted in converted_events {
                                        if !stream_send(stream_tx, Bytes::from(converted)).await {
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

    // 处理 buffer 中残余的最后一个事件
    if !state.buffer.is_empty() && !state.buffer.iter().all(|b| *b == b'\n' || *b == b'\r') {
        if let Ok(text) = std::str::from_utf8(&state.buffer)
            && let Some(source) = extract_usage_from_sse(text, ctx.upstream_endpoint)
        {
            apply_sse_usage(source, &mut state.last_usage, &mut state.input_usage);
        }
        let mut is_error_event = false;
        if state.stream_error.is_none()
            && let Ok(text) = std::str::from_utf8(&state.buffer)
            && let Some(error) = extract_error_from_sse(text, ctx.upstream_endpoint)
        {
            state.stream_error = Some(error);
            is_error_event = true;
        }
        if !is_error_event {
            if let Ok(Some(llm_stream)) =
                RelayPipeline::decode_stream_event(ctx.upstream_endpoint, &state.buffer)
            {
                match converter.convert(&llm_stream) {
                    Ok(converted_events) => {
                        for converted in converted_events {
                            stream_send(stream_tx, Bytes::from(converted)).await;
                        }
                    }
                    Err(e) => {
                        tracing::error!("Stream conversion error (drain): {}", e);
                    }
                }
            }
        } else if let Some(error) = state.stream_error.as_deref() {
            stream_send(
                stream_tx,
                Bytes::from(format_stream_error_event(error, ctx.client_endpoint)),
            )
            .await;
        }
    }

    // 发送流结束事件
    match converter.finish() {
        Ok(finish_events) => {
            for event_bytes in finish_events {
                stream_send(stream_tx, Bytes::from(event_bytes)).await;
            }
        }
        Err(e) => {
            tracing::error!("Stream finish error: {}", e);
        }
    }
}

/// passthrough 路径：透传原始字节（OpenAiChat + thinking 时做 decode→observe→reencode 改写）。
pub(super) async fn run_passthrough_loop<S>(
    state: &mut StreamCollectState,
    ctx: &LoopCtx<'_>,
    mut stream: std::pin::Pin<&mut S>,
    stream_tx: &Sender<Result<Bytes, Infallible>>,
) where
    S: futures::Stream<Item = Result<Bytes, reqwest::Error>>,
{
    // thinking passthrough 改写：仅 OpenAiChat（DeepSeek/QwQ 直连客户端，<think> 混 content）。
    // 其他协议 passthrough 透传（其上游用结构化 thinking_delta，且 inbound encode 会破坏流结构）。
    let thinking_pt_rewrite =
        state.thinking_processor.is_some() && *ctx.upstream_endpoint == EndpointType::OpenAiChat;

    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(bytes) => {
                state.buffer.extend_from_slice(&bytes);

                let mut client_disconnected = false;
                while let Some(event_end) = find_sse_boundary(&state.buffer) {
                    let event_bytes = state.buffer[..event_end].to_vec();
                    state.buffer = state.buffer[event_end..].to_vec();

                    if event_bytes.iter().all(|b| *b == b'\n' || *b == b'\r') {
                        continue;
                    }

                    // TTFT：在第一个有效 SSE 事件处记录（比"第一个 chunk"更精确）
                    if !state.first_token_seen {
                        state.ttft_ms = Some(ctx.start_time.elapsed().as_millis() as i32);
                        state.first_token_seen = true;
                    }

                    if let Ok(text) = std::str::from_utf8(&event_bytes) {
                        if let Some(source) =
                            extract_usage_from_sse(text, ctx.upstream_endpoint)
                        {
                            apply_sse_usage(source, &mut state.last_usage, &mut state.input_usage);
                        }
                        if state.stream_error.is_none()
                            && let Some(error) =
                                extract_error_from_sse(text, ctx.upstream_endpoint)
                        {
                            state.stream_error = Some(error);
                        }
                        // 落库收集：thinking 改写时 reasoning 由 processor 累积，collect 用 dummy 避免重复
                        if thinking_pt_rewrite {
                            let mut _dummy_reasoning = String::new();
                            collect_sse_content(
                                text,
                                ctx.upstream_endpoint,
                                &mut state.collected_text,
                                &mut _dummy_reasoning,
                                &mut state.collected_tool_calls,
                            );
                        } else {
                            collect_sse_content(
                                text,
                                ctx.upstream_endpoint,
                                &mut state.collected_text,
                                &mut state.collected_reasoning,
                                &mut state.collected_tool_calls,
                            );
                        }
                    }

                    // 转发：thinking 改写时 decode→observe→reencode；否则透传原始字节
                    let send_bytes = if thinking_pt_rewrite {
                        rewrite_thinking_passthrough(
                            ctx.upstream_endpoint,
                            &event_bytes,
                            state.thinking_processor.as_mut(),
                        )
                    } else {
                        Bytes::from(event_bytes)
                    };
                    if !stream_send(stream_tx, send_bytes).await {
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
    if !state.buffer.is_empty()
        && !state.buffer.iter().all(|b| *b == b'\n' || *b == b'\r')
        && let Ok(text) = std::str::from_utf8(&state.buffer)
    {
        if let Some(source) = extract_usage_from_sse(text, ctx.upstream_endpoint) {
            apply_sse_usage(source, &mut state.last_usage, &mut state.input_usage);
        }
        if state.stream_error.is_none()
            && let Some(error) = extract_error_from_sse(text, ctx.upstream_endpoint)
        {
            state.stream_error = Some(error);
        }
        // 残余事件：thinking 改写时 reasoning 由 processor，collect 用 dummy（残余通常无 content/reasoning）
        if thinking_pt_rewrite {
            let mut _dummy_reasoning = String::new();
            collect_sse_content(
                text,
                ctx.upstream_endpoint,
                &mut state.collected_text,
                &mut _dummy_reasoning,
                &mut state.collected_tool_calls,
            );
        } else {
            collect_sse_content(
                text,
                ctx.upstream_endpoint,
                &mut state.collected_text,
                &mut state.collected_reasoning,
                &mut state.collected_tool_calls,
            );
        }
    }
}
