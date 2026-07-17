//! 从 stream_executor.rs 拆出（行为不变）：流式 api_key 迭代 + 针对性熔断。
//!
//! 单轮遍历渠道所有可用 key 执行流式代理请求；per-key 熔断跳过已熔断 key；
//! 返回 [`StreamKeyLoopOutcome`] 让上层判断是否要做"503 同渠道退避重试"。

use crate::error::proxy::ProxyError;
use crate::llm::relay::run::{
    RelayAttemptError, RelayCandidate, RelayStreamAttemptResult, RelayStreamSuccess,
};
use crate::llm::scheduler::selector::SelectionResult;

use super::stream_executor::ProxyStreamRelayExecutor;

/// 流式 [`run_key_stream_loop`] 的返回结果。
pub(super) enum StreamKeyLoopOutcome {
    Success(RelayStreamSuccess),
    NonKeyRetryableError(RelayAttemptError),
    AllKeysTried {
        last_error: Option<RelayAttemptError>,
        all_executed_503: bool,
    },
}

impl StreamKeyLoopOutcome {
    pub(super) fn should_retry_503(&self) -> bool {
        matches!(
            self,
            StreamKeyLoopOutcome::AllKeysTried {
                all_executed_503: true,
                ..
            }
        )
    }

    pub(super) fn into_stream_result(self) -> RelayStreamAttemptResult {
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

/// 单轮遍历所有 key 执行流式代理请求。
///
/// 返回 [`StreamKeyLoopOutcome`] 让上层判断是否要做"503 同渠道退避重试"。
pub(super) async fn run_key_stream_loop(
    executor: &ProxyStreamRelayExecutor,
    selection: &SelectionResult,
    candidate: &RelayCandidate,
) -> StreamKeyLoopOutcome {
    let api_key_attempts = executor.state.api_key_attempts(&selection.channel);
    let mut last_error = None;
    let mut executed_count = 0u32;
    let mut all_executed_503 = true;

    for upstream_api_key in &api_key_attempts {
        let key_hint = selection.channel.key_hint(upstream_api_key);

        // per-key 熔断：该 key 已熔断则跳过，直接试下一个 key
        let (tripped, _) = executor
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

        let mut local_attempts = executor
            .attempt_stats
            .lock()
            .map(|mut stats| std::mem::take(&mut *stats))
            .unwrap_or_default();
        let queue_permit = executor
            .queue_permit
            .lock()
            .expect("queue_permit mutex poisoned")
            .take();

        let result = executor
            .execute_proxy_stream(
                upstream_api_key,
                &key_hint,
                selection,
                &mut local_attempts,
                queue_permit,
            )
            .await;

        if let Ok(mut stats) = executor.attempt_stats.lock() {
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
                    executor
                        .state
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
