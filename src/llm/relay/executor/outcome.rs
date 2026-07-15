//! run_key_loop 的返回结果（成功 / 非 key-retryable 失败 / 所有 key 已尝试）。
//! 把三种情况显式区分，方便上层决策是否做 503 同渠道退避重试。

use crate::llm::relay::run::{RelayAttemptError, RelayAttemptResult};

/// run_key_loop 的返回结果。把"成功 / 非 key-retryable 失败 / 所有 key 已尝试"
/// 三种情况显式区分开，方便上层决策是否做 503 同渠道退避重试。
pub(super) enum KeyLoopOutcome {
    Success(String),
    NonKeyRetryableError(RelayAttemptError),
    AllKeysTried {
        last_error: Option<RelayAttemptError>,
        /// 所有真正执行过的 key 都返回 503
        all_executed_503: bool,
    },
}

impl KeyLoopOutcome {
    /// 是否应该做 503 同渠道退避重试
    pub(super) fn should_retry_503(&self) -> bool {
        matches!(
            self,
            KeyLoopOutcome::AllKeysTried {
                all_executed_503: true,
                ..
            }
        )
    }

    /// 转换为 RelayAttemptResult
    pub(super) fn into_result(self) -> RelayAttemptResult {
        match self {
            KeyLoopOutcome::Success(body) => RelayAttemptResult {
                response: Ok(body),
                response_written: false,
            },
            KeyLoopOutcome::NonKeyRetryableError(err)
            | KeyLoopOutcome::AllKeysTried {
                last_error: Some(err),
                ..
            } => RelayAttemptResult {
                response: Err(err),
                response_written: false,
            },
            KeyLoopOutcome::AllKeysTried {
                last_error: None, ..
            } => RelayAttemptResult {
                response: Err(RelayAttemptError::new(500, "all api keys exhausted")),
                response_written: false,
            },
        }
    }
}

#[cfg(test)]
mod outcome_tests {
    use super::*;

    #[test]
    fn key_loop_outcome_all_503_triggers_retry() {
        let outcome = KeyLoopOutcome::AllKeysTried {
            last_error: Some(RelayAttemptError::new(503, "upstream overloaded")),
            all_executed_503: true,
        };
        assert!(
            outcome.should_retry_503(),
            "所有执行过的 key 都返回 503 时应触发同渠道退避重试"
        );
    }

    #[test]
    fn key_loop_outcome_mixed_errors_no_retry() {
        // 有执行过的 key 返回 429/500，非全部 503 → 不触发重试
        let outcome = KeyLoopOutcome::AllKeysTried {
            last_error: Some(RelayAttemptError::new(500, "upstream overloaded")),
            all_executed_503: false,
        };
        assert!(!outcome.should_retry_503());
    }

    #[test]
    fn key_loop_outcome_no_execution_no_retry() {
        // 所有 key 都被熔断跳过，没有任何执行 → 不触发重试
        let outcome = KeyLoopOutcome::AllKeysTried {
            last_error: None,
            all_executed_503: false,
        };
        assert!(!outcome.should_retry_503());
    }

    #[test]
    fn key_loop_outcome_success_no_retry() {
        let outcome = KeyLoopOutcome::Success("ok".to_string());
        assert!(!outcome.should_retry_503());
    }

    #[test]
    fn key_loop_outcome_into_result_carries_last_error() {
        let outcome = KeyLoopOutcome::AllKeysTried {
            last_error: Some(RelayAttemptError::new(503, "upstream overloaded")),
            all_executed_503: false,
        };
        let result = outcome.into_result();
        match result.response {
            Err(e) => assert_eq!(e.status_code, 503),
            Ok(_) => panic!("应返回 last_error"),
        }
    }

    #[test]
    fn key_loop_outcome_all_keys_tried_no_error_falls_back_to_500() {
        let outcome = KeyLoopOutcome::AllKeysTried {
            last_error: None,
            all_executed_503: false,
        };
        let result = outcome.into_result();
        match result.response {
            Err(e) => assert_eq!(e.status_code, 500),
            Ok(_) => panic!("应返回 all api keys exhausted"),
        }
    }
}
