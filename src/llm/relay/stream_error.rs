//! 流式执行外围辅助（Step D 从 stream_executor 拆出，行为不变）。
//!
//! - `decrement_active_once`：错误路径 CAS 一次 decrement（防双扣）
//! - `rewrite_thinking_passthrough`：thinking passthrough（OpenAiChat）decode→observe→reencode
//! - `StreamPanicGuard`：spawn task panic 兜底，Drop 时写简化失败日志保证不漏记

use axum::body::Bytes;

use crate::domain::channel::EndpointType;
use crate::app_state::AppState;
use crate::service::stats::recorder::RequestRecord;
use crate::llm::relay::converter::RelayPipeline;

/// 仅在未被 decrement 过时执行一次 decrement（用于流式请求的错误路径）
pub(super) fn decrement_active_once(
    state: &AppState,
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

/// thinking passthrough 改写：decode 上游 SSE → processor.observe 改写 → reencode（OpenAiChat serde）。
/// 仅 passthrough + thinking 启用 + endpoint==OpenAiChat 时调用。decode 返回 None
/// (`[DONE]`/心跳) 或失败时透传原字节，保证不影响非 content 事件与 usage/finish 结构。
pub(super) fn rewrite_thinking_passthrough(
    endpoint: &EndpointType,
    event_bytes: &[u8],
    processor: Option<&mut Box<dyn crate::llm::plugin::StreamResponseProcessor>>,
) -> Bytes {
    let Some(processor) = processor else {
        return Bytes::from(event_bytes.to_vec());
    };
    match RelayPipeline::decode_stream_event(endpoint, event_bytes) {
        Ok(Some(mut llm_stream)) => {
            processor.observe(&mut llm_stream);
            match serde_json::to_string(&llm_stream) {
                Ok(data) => Bytes::from(format!("data: {}\n\n", data).into_bytes()),
                Err(e) => {
                    tracing::warn!("thinking passthrough reencode 失败，透传原字节: {}", e);
                    Bytes::from(event_bytes.to_vec())
                }
            }
        }
        _ => Bytes::from(event_bytes.to_vec()),
    }
}

/// spawn task 的 panic 兜底 guard：正常路径 disarm，panic 时由 Drop 写一条简化失败日志，
/// 保证请求不漏记（"CC 已失败但 usage_logs 缺失"的核心修复之一）。
pub(super) struct StreamPanicGuard {
    state: AppState,
    record: RequestRecord,
    armed: bool,
}

impl StreamPanicGuard {
    pub(super) fn new(state: AppState, record: RequestRecord) -> Self {
        Self {
            state,
            record,
            armed: true,
        }
    }

    pub(super) fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for StreamPanicGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }

        // catch_unwind 兜底：Drop 里任何 panic 都不能冒泡，否则就是二次 panic → 进程 abort
        let state = self.state.clone();
        let record = self.record.clone();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            tracing::error!(
                request_id = %record.request_id.as_deref().unwrap_or(""),
                model = %record.requested_model,
                "流式 spawn task panic，写兜底失败日志"
            );

            // 独立 tokio::spawn：使用 detached context，不受客户端断开影响
            tokio::spawn(async move {
                // 失败日志：status=500、token=0、空 attempts，但保留 request_id/model/api_key 等关键字段
                let failed_record = RequestRecord {
                    status_code: Some(500),
                    error_message: Some("流式处理 task 内部异常（已兜底记录）".to_string()),
                    input_tokens: 0,
                    output_tokens: 0,
                    cache_read_tokens: 0,
                    cache_creation_tokens: 0,
                    cost: None,
                    latency_ms: None,
                    ttft_ms: None,
                    attempts: vec![],
                    ..record
                };
                // 10s 超时，避免 DB 卡死拖住 cleanup task
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    state.stats_recorder.record_request(failed_record),
                )
                .await;
            });
        }));
    }
}
