use async_trait::async_trait;
use galaxy_router::error::proxy::ProxyError;
use galaxy_router::llm::relay::run::{
    RelayAttemptError, RelayAttemptExecutor, RelayAttemptResult, RelayCandidate, RelayRequest,
    RelayRun,
};
use galaxy_router::llm::scheduler::capacity::ChannelCapacityManager;
use galaxy_router::llm::scheduler::trace::AttemptStatus;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct FakeExecutor {
    results: Arc<Mutex<VecDeque<Result<String, RelayAttemptError>>>>,
    calls: Arc<Mutex<Vec<String>>>,
    /// 如果 Some(idx)，第 idx 次调用会设置 response_written=true
    response_written_at: Arc<Mutex<Option<usize>>>,
    /// 标记为不可用（熔断）的渠道列表
    unavailable_channels: Arc<Mutex<Vec<String>>>,
    /// 记录 on_attempt_failed 回调：(channel_id, status_code, is_server_error)
    failure_feedback: Arc<Mutex<Vec<(String, u16, bool)>>>,
}

impl FakeExecutor {
    fn with_results(results: Vec<Result<&str, RelayAttemptError>>) -> Self {
        Self {
            results: Arc::new(Mutex::new(
                results
                    .into_iter()
                    .map(|r| r.map(str::to_string))
                    .collect::<VecDeque<_>>(),
            )),
            calls: Arc::new(Mutex::new(Vec::new())),
            response_written_at: Arc::new(Mutex::new(None)),
            unavailable_channels: Arc::new(Mutex::new(Vec::new())),
            failure_feedback: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// 让第 idx 次调用（0-based）设置 response_written=true
    fn with_response_written_at(self, idx: usize) -> Self {
        *self.response_written_at.lock().expect("mutex") = Some(idx);
        self
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("calls mutex poisoned").clone()
    }

    fn call_count(&self) -> usize {
        self.calls.lock().expect("calls mutex poisoned").len()
    }
}

#[async_trait]
impl RelayAttemptExecutor for FakeExecutor {
    async fn is_channel_available(&self, channel_id: &str) -> bool {
        !self
            .unavailable_channels
            .lock()
            .expect("mutex")
            .iter()
            .any(|c| c == channel_id)
    }

    async fn on_attempt_failed(&self, channel_id: &str, error: &ProxyError) {
        let status_code = match error {
            ProxyError::UpstreamError { status, .. } => status.as_u16(),
            _ => 502,
        };
        let is_server_error = matches!(
            error.classify(),
            galaxy_router::error::proxy::ErrorClass::UpstreamRetryable
        );
        self.failure_feedback.lock().expect("mutex").push((
            channel_id.to_string(),
            status_code,
            is_server_error,
        ));
    }

    async fn execute(
        &self,
        request: &RelayRequest,
        candidate: &RelayCandidate,
    ) -> RelayAttemptResult {
        let call_idx = self.call_count();
        self.calls
            .lock()
            .expect("calls mutex poisoned")
            .push(format!(
                "{}:{}",
                request.requested_model, candidate.channel_id
            ));
        let result = self
            .results
            .lock()
            .expect("results mutex poisoned")
            .pop_front()
            .unwrap_or_else(|| Err(RelayAttemptError::new(500, "no fake result")));

        let response_written = self
            .response_written_at
            .lock()
            .expect("mutex")
            .map(|idx| call_idx == idx)
            .unwrap_or(false);

        RelayAttemptResult {
            response: result,
            response_written,
        }
    }
}

fn candidate(id: &str, score: f64) -> RelayCandidate {
    RelayCandidate {
        channel_id: id.to_string(),
        channel_name: format!("channel-{id}"),
        max_concurrency: 1,
        score,
        sticky: false,
        target_model: "gpt-4o".to_string(),
        route_id: None,
    }
}

fn sticky_candidate(id: &str, score: f64) -> RelayCandidate {
    RelayCandidate {
        channel_id: id.to_string(),
        channel_name: format!("channel-{id}"),
        max_concurrency: 1,
        score,
        sticky: true,
        target_model: "gpt-4o".to_string(),
        route_id: None,
    }
}

#[tokio::test]
async fn relay_run_returns_first_success_and_records_trace() {
    let executor = FakeExecutor::with_results(vec![Ok("ok-from-a")]);
    let run = RelayRun::new(ChannelCapacityManager::new(), executor.clone());

    let outcome = run
        .execute(
            RelayRequest::new("gpt-4o"),
            vec![candidate("a", 10.0), candidate("b", 1.0)],
        )
        .await;

    assert!(outcome.is_success());
    assert_eq!(outcome.response.as_deref(), Some("ok-from-a"));
    assert_eq!(outcome.selected_channel_id.as_deref(), Some("a"));
    assert_eq!(executor.calls(), vec!["gpt-4o:a"]);
    assert_eq!(outcome.attempts.len(), 1);
    assert_eq!(outcome.attempts[0].attempt_no, 1);
    assert_eq!(outcome.attempts[0].status, AttemptStatus::Success);
    assert_eq!(outcome.attempts[0].channel_id.as_deref(), Some("a"));
    assert_eq!(outcome.attempts[0].score, Some(10.0));
}

#[tokio::test]
async fn relay_run_retries_next_candidate_after_failure() {
    let executor = FakeExecutor::with_results(vec![
        Err(RelayAttemptError::new(429, "quota exceeded")),
        Ok("ok-from-b"),
    ]);
    let run = RelayRun::new(ChannelCapacityManager::new(), executor.clone());

    let outcome = run
        .execute(
            RelayRequest::new("gpt-4o"),
            vec![candidate("a", 10.0), candidate("b", 9.0)],
        )
        .await;

    assert!(outcome.is_success());
    assert_eq!(outcome.response.as_deref(), Some("ok-from-b"));
    assert_eq!(outcome.selected_channel_id.as_deref(), Some("b"));
    assert_eq!(executor.calls(), vec!["gpt-4o:a", "gpt-4o:b"]);
    assert_eq!(outcome.attempts.len(), 2);
    assert_eq!(outcome.attempts[0].status, AttemptStatus::Failed);
    assert_eq!(
        outcome.attempts[0].reason.as_deref(),
        Some("quota exceeded")
    );
    assert_eq!(outcome.attempts[1].status, AttemptStatus::Success);
}

#[tokio::test]
async fn relay_run_returns_failure_when_all_candidates_fail() {
    let executor = FakeExecutor::with_results(vec![
        Err(RelayAttemptError::new(500, "upstream down")),
        Err(RelayAttemptError::new(503, "backup down")),
    ]);
    let run = RelayRun::new(ChannelCapacityManager::new(), executor.clone());

    let outcome = run
        .execute(
            RelayRequest::new("gpt-4o"),
            vec![candidate("a", 10.0), candidate("b", 9.0)],
        )
        .await;

    assert!(!outcome.is_success());
    assert_eq!(outcome.status_code, 503);
    assert_eq!(outcome.error_message.as_deref(), Some("backup down"));
    assert_eq!(outcome.selected_channel_id, None);
    assert_eq!(outcome.attempts.len(), 2);
    assert_eq!(outcome.attempts[0].status, AttemptStatus::Failed);
    assert_eq!(outcome.attempts[1].status, AttemptStatus::Failed);
}

#[tokio::test]
async fn relay_run_skips_candidate_when_capacity_is_full_and_releases_after_success() {
    let capacity = ChannelCapacityManager::new();
    let held = capacity.try_acquire("a", 1).expect("hold capacity for a");
    let executor = FakeExecutor::with_results(vec![Ok("ok-from-b")]);
    let run = RelayRun::new(capacity.clone(), executor.clone());

    let outcome = run
        .execute(
            RelayRequest::new("gpt-4o"),
            vec![candidate("a", 10.0), candidate("b", 9.0)],
        )
        .await;

    assert!(outcome.is_success());
    assert_eq!(executor.calls(), vec!["gpt-4o:b"]);
    assert_eq!(outcome.attempts[0].status, AttemptStatus::Skipped);
    assert_eq!(outcome.attempts[0].reason.as_deref(), Some("capacity full"));
    assert_eq!(outcome.attempts[1].status, AttemptStatus::Success);

    drop(held);
}

/// P3.3: 当 executor 标记 response_written=true 时，RelayRun 不再尝试后续候选
#[tokio::test]
async fn relay_run_stops_on_response_written() {
    // 3 个候选，第 1 个失败且标记 response_written=true（模拟流式已开始写入）
    // 即使后续还有候选，也不应重试
    let executor = FakeExecutor::with_results(vec![Err(RelayAttemptError::new(
        500,
        "stream started then failed",
    ))])
    .with_response_written_at(0);

    let run = RelayRun::new(ChannelCapacityManager::new(), executor.clone());

    let outcome = run
        .execute(
            RelayRequest::new("gpt-4o"),
            vec![
                candidate("a", 10.0),
                candidate("b", 9.0),
                candidate("c", 8.0),
            ],
        )
        .await;

    // 只调用了 a，没有重试 b 和 c
    assert_eq!(executor.calls(), vec!["gpt-4o:a"]);
    assert_eq!(outcome.attempts.len(), 1);
    assert_eq!(outcome.attempts[0].status, AttemptStatus::Failed);
    assert_eq!(outcome.status_code, 500);
}

/// P5.2: 熔断渠道记录 CircuitBreak trace，跳过执行，尝试下一个候选
#[tokio::test]
async fn relay_run_skips_circuit_broken_candidate_and_records_trace() {
    let executor = FakeExecutor::with_results(vec![Ok("ok-from-b")]);
    // 标记渠道 "a" 为不可用（熔断）
    executor
        .unavailable_channels
        .lock()
        .expect("mutex")
        .push("a".to_string());

    let run = RelayRun::new(ChannelCapacityManager::new(), executor.clone());

    let outcome = run
        .execute(
            RelayRequest::new("gpt-4o"),
            vec![candidate("a", 10.0), candidate("b", 9.0)],
        )
        .await;

    // a 被熔断跳过，b 成功
    assert!(outcome.is_success());
    assert_eq!(outcome.response.as_deref(), Some("ok-from-b"));
    assert_eq!(executor.calls(), vec!["gpt-4o:b"]);

    // trace: a=circuit_break, b=success
    assert_eq!(outcome.attempts.len(), 2);
    assert_eq!(outcome.attempts[0].status, AttemptStatus::CircuitBreak);
    assert_eq!(outcome.attempts[0].channel_id.as_deref(), Some("a"));
    assert_eq!(outcome.attempts[0].reason.as_deref(), Some("circuit open"));
    assert_eq!(outcome.attempts[1].status, AttemptStatus::Success);
    assert_eq!(outcome.attempts[1].channel_id.as_deref(), Some("b"));
}

/// P5.4: 失败尝试后触发 on_attempt_failed 回调，携带正确的错误信息
#[tokio::test]
async fn relay_run_calls_failure_feedback_on_each_failed_attempt() {
    let executor = FakeExecutor::with_results(vec![
        Err(RelayAttemptError::new(429, "rate limited")),
        Err(RelayAttemptError::new(500, "internal error")),
        Ok("ok-from-c"),
    ]);
    let run = RelayRun::new(ChannelCapacityManager::new(), executor.clone());

    let outcome = run
        .execute(
            RelayRequest::new("gpt-4o"),
            vec![
                candidate("a", 10.0),
                candidate("b", 9.0),
                candidate("c", 8.0),
            ],
        )
        .await;

    assert!(outcome.is_success());
    assert_eq!(executor.calls(), vec!["gpt-4o:a", "gpt-4o:b", "gpt-4o:c"]);

    // 验证 failure_feedback 记录了两次失败
    let feedback = executor.failure_feedback.lock().expect("mutex").clone();
    assert_eq!(feedback.len(), 2);

    // 第 1 次: channel a, 429, 非 server error
    assert_eq!(feedback[0].0, "a");
    assert_eq!(feedback[0].1, 429);
    assert!(!feedback[0].2);

    // 第 2 次: channel b, 500, 是 server error
    assert_eq!(feedback[1].0, "b");
    assert_eq!(feedback[1].1, 500);
    assert!(feedback[1].2);
}

/// P5.1: sticky 候选在 trace 中标记 sticky=true
#[tokio::test]
async fn relay_run_records_sticky_flag_in_success_trace() {
    let executor = FakeExecutor::with_results(vec![Ok("ok-from-a")]);
    let run = RelayRun::new(ChannelCapacityManager::new(), executor.clone());

    let outcome = run
        .execute(
            RelayRequest::new("gpt-4o"),
            vec![sticky_candidate("a", 10.0), candidate("b", 9.0)],
        )
        .await;

    assert!(outcome.is_success());
    assert_eq!(outcome.attempts.len(), 1);
    assert!(
        outcome.attempts[0].sticky,
        "sticky candidate should be marked"
    );
}

/// P5.1: sticky 候选被熔断时，trace 同时记录 sticky=true 和 CircuitBreak
#[tokio::test]
async fn relay_run_records_sticky_flag_when_circuit_broken() {
    let executor = FakeExecutor::with_results(vec![Ok("ok-from-b")]);
    executor
        .unavailable_channels
        .lock()
        .expect("mutex")
        .push("a".to_string());

    let run = RelayRun::new(ChannelCapacityManager::new(), executor.clone());

    let outcome = run
        .execute(
            RelayRequest::new("gpt-4o"),
            vec![sticky_candidate("a", 10.0), candidate("b", 9.0)],
        )
        .await;

    assert!(outcome.is_success());
    assert_eq!(outcome.attempts.len(), 2);

    // sticky 候选被熔断
    assert_eq!(outcome.attempts[0].status, AttemptStatus::CircuitBreak);
    assert!(
        outcome.attempts[0].sticky,
        "sticky candidate should be marked even when circuit broken"
    );

    // 非 sticky 候选成功
    assert_eq!(outcome.attempts[1].status, AttemptStatus::Success);
    assert!(!outcome.attempts[1].sticky);
}
