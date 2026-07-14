use async_trait::async_trait;
use futures::StreamExt;
use std::time::Instant;

use crate::error::proxy::ProxyError;
use crate::scheduler::capacity::{ChannelCapacityManager, ChannelCapacityPermit};
use crate::scheduler::trace::{AttemptTrace, AttemptTraceBuilder};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayRequest {
    pub requested_model: String,
}

impl RelayRequest {
    pub fn new(requested_model: impl Into<String>) -> Self {
        Self {
            requested_model: requested_model.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RelayCandidate {
    pub channel_id: String,
    pub channel_name: String,
    pub max_concurrency: u32,
    pub score: f64,
    /// 是否为 session sticky 候选
    pub sticky: bool,
    /// 上游使用的模型名（来自 group_item.model_name 或直接映射）
    pub target_model: String,
    /// 所属分组 ID（直接渠道为 None）
    pub route_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RelayAttemptError {
    pub status_code: u16,
    pub message: String,
    /// 原始 ProxyError（用于下游判断是否 key-retryable 等）。
    /// 仅当错误来自上游错误时存在；来自构建/转换阶段时为 None。
    pub proxy_error: Option<ProxyError>,
}

impl RelayAttemptError {
    pub fn new(status_code: u16, message: impl Into<String>) -> Self {
        Self {
            status_code,
            message: message.into(),
            proxy_error: None,
        }
    }

    /// 构造带原始 ProxyError 的失败结果（上游错误路径）。
    /// 保留原始分类（KeyRetryable / UpstreamRetryable 等）供下游决策使用。
    pub fn from_proxy_error(error: ProxyError) -> Self {
        let status_code = match &error {
            ProxyError::UpstreamError { status, .. } => status.as_u16(),
            ProxyError::NoAvailableChannel(_) => 503,
            ProxyError::ModelNotSupported(_) => 400,
            ProxyError::ModelNotFound(_) => 404,
            ProxyError::DatabaseError(_) => 500,
            ProxyError::ChannelNotFound(_) => 404,
            ProxyError::RequestError(_) => 502,
            ProxyError::TransformError(_) => 500,
        };
        // message 需包含错误正文，避免仅显示 "上游错误: 503" 丢失诊断信息
        let message = match &error {
            ProxyError::UpstreamError { status, body } => {
                let truncated = &body[..body.len().min(500)];
                format!("上游错误: {} {}", status, truncated)
            }
            other => other.to_string(),
        };
        Self {
            status_code,
            message,
            proxy_error: Some(error),
        }
    }
}

/// executor 单次尝试的返回结果
pub struct RelayAttemptResult {
    /// 成功时为响应文本，失败时为错误
    pub response: Result<String, RelayAttemptError>,
    /// 流式场景下是否已开始写入响应（为 true 时不再重试后续候选）
    pub response_written: bool,
}

#[async_trait]
pub trait RelayAttemptExecutor: Send + Sync + Clone + 'static {
    /// 检查渠道是否可用（熔断器检查）。默认返回 true（始终可用）
    async fn is_channel_available(&self, _channel_id: &str) -> bool {
        true
    }

    /// 失败尝试后的反馈回调。默认空操作
    async fn on_attempt_failed(&self, _channel_id: &str, _error: &ProxyError) {}

    async fn execute(
        &self,
        request: &RelayRequest,
        candidate: &RelayCandidate,
    ) -> RelayAttemptResult;
}

#[derive(Debug, Clone)]
pub struct RelayRun<E> {
    capacity: ChannelCapacityManager,
    executor: E,
}

impl<E> RelayRun<E>
where
    E: RelayAttemptExecutor,
{
    pub fn new(capacity: ChannelCapacityManager, executor: E) -> Self {
        Self { capacity, executor }
    }

    pub async fn execute(
        &self,
        request: RelayRequest,
        candidates: Vec<RelayCandidate>,
    ) -> RelayRunOutcome {
        let mut trace_builder = AttemptTraceBuilder::new(request.requested_model.clone());
        let mut last_error: Option<RelayAttemptError> = None;

        for candidate in candidates {
            // 熔断检查：渠道不可用时跳过，记录 CircuitBreak trace
            if !self
                .executor
                .is_channel_available(&candidate.channel_id)
                .await
            {
                trace_builder
                    .circuit_break()
                    .channel(&candidate.channel_id, &candidate.channel_name)
                    .reason("circuit open")
                    .sticky(candidate.sticky)
                    .score(candidate.score)
                    .finish();
                continue;
            }

            let Some(_permit) = self
                .capacity
                .try_acquire(&candidate.channel_id, candidate.max_concurrency)
            else {
                trace_builder
                    .skipped()
                    .channel(&candidate.channel_id, &candidate.channel_name)
                    .reason("capacity full")
                    .sticky(candidate.sticky)
                    .score(candidate.score)
                    .finish();
                continue;
            };

            let started = Instant::now();
            let result = self.executor.execute(&request, &candidate).await;
            match result.response {
                Ok(response) => {
                    trace_builder
                        .success()
                        .channel(&candidate.channel_id, &candidate.channel_name)
                        .duration_ms(elapsed_ms(started))
                        .sticky(candidate.sticky)
                        .score(candidate.score)
                        .finish();
                    return RelayRunOutcome {
                        response: Some(response),
                        selected_channel_id: Some(candidate.channel_id),
                        status_code: 200,
                        error_message: None,
                        attempts: trace_builder.finish_all(),
                    };
                }
                Err(error) => {
                    trace_builder
                        .failed()
                        .channel(&candidate.channel_id, &candidate.channel_name)
                        .reason(error.message.clone())
                        .duration_ms(elapsed_ms(started))
                        .sticky(candidate.sticky)
                        .score(candidate.score)
                        .finish();

                    // 反馈失败信息给调度器（用于更新 error_rate、熔断等）
                    let proxy_error_for_feedback = error.proxy_error.as_ref().cloned().unwrap_or_else(|| {
                        ProxyError::UpstreamError {
                            status: axum::http::StatusCode::from_u16(error.status_code)
                                .unwrap_or(axum::http::StatusCode::BAD_GATEWAY),
                            body: error.message.clone(),
                        }
                    });
                    self.executor
                        .on_attempt_failed(&candidate.channel_id, &proxy_error_for_feedback)
                        .await;

                    last_error = Some(error);

                    // 流式场景下响应已开始写入，停止重试
                    if result.response_written {
                        break;
                    }
                }
            }
        }

        let (status_code, error_message) = last_error
            .map(|e| (e.status_code, Some(e.message)))
            .unwrap_or((503, Some("no available candidate".to_string())));

        RelayRunOutcome {
            response: None,
            selected_channel_id: None,
            status_code,
            error_message,
            attempts: trace_builder.finish_all(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RelayRunOutcome {
    pub response: Option<String>,
    pub selected_channel_id: Option<String>,
    pub status_code: u16,
    pub error_message: Option<String>,
    #[allow(dead_code)]
    pub attempts: Vec<AttemptTrace>,
}

#[allow(dead_code)]
impl RelayRunOutcome {
    pub fn is_success(&self) -> bool {
        self.response.is_some() && (200..300).contains(&self.status_code)
    }
}

fn elapsed_ms(started: Instant) -> i64 {
    started.elapsed().as_millis().min(i64::MAX as u128) as i64
}

/// 流式响应体类型。
pub type RelayStreamBody = std::pin::Pin<
    Box<
        dyn futures::Stream<Item = Result<axum::body::Bytes, std::convert::Infallible>>
            + Send
            + 'static,
    >,
>;

pub struct RelayStreamSuccess {
    pub status: axum::http::StatusCode,
    pub stream: RelayStreamBody,
    pub content_type: String,
    /// Keeps the scheduler capacity slot alive for the stream lifetime.
    pub _capacity_permit: Option<ChannelCapacityPermit>,
}

pub struct RelayStreamAttemptResult {
    pub response: Result<RelayStreamSuccess, RelayAttemptError>,
    /// 流式场景下是否已开始写入响应（为 true 时不再重试后续候选）。
    pub response_written: bool,
}

#[async_trait]
pub trait RelayStreamAttemptExecutor: Send + Sync + Clone + 'static {
    async fn is_channel_available(&self, _channel_id: &str) -> bool {
        true
    }

    async fn on_attempt_failed(&self, _channel_id: &str, _error: &ProxyError) {}

    async fn execute_stream(
        &self,
        request: &RelayRequest,
        candidate: &RelayCandidate,
    ) -> RelayStreamAttemptResult;
}

#[derive(Debug, Clone)]
pub struct RelayStreamRun<E> {
    capacity: ChannelCapacityManager,
    executor: E,
}

impl<E> RelayStreamRun<E>
where
    E: RelayStreamAttemptExecutor,
{
    pub fn new(capacity: ChannelCapacityManager, executor: E) -> Self {
        Self { capacity, executor }
    }

    pub async fn execute(
        &self,
        request: RelayRequest,
        candidates: Vec<RelayCandidate>,
    ) -> RelayStreamRunOutcome {
        let mut trace_builder = AttemptTraceBuilder::new(request.requested_model.clone());
        let mut last_error: Option<RelayAttemptError> = None;

        for candidate in candidates {
            if !self
                .executor
                .is_channel_available(&candidate.channel_id)
                .await
            {
                trace_builder
                    .circuit_break()
                    .channel(&candidate.channel_id, &candidate.channel_name)
                    .reason("circuit open")
                    .sticky(candidate.sticky)
                    .score(candidate.score)
                    .finish();
                continue;
            }

            let Some(permit) = self
                .capacity
                .try_acquire(&candidate.channel_id, candidate.max_concurrency)
            else {
                trace_builder
                    .skipped()
                    .channel(&candidate.channel_id, &candidate.channel_name)
                    .reason("capacity full")
                    .sticky(candidate.sticky)
                    .score(candidate.score)
                    .finish();
                continue;
            };

            let started = Instant::now();
            let result = self.executor.execute_stream(&request, &candidate).await;
            match result.response {
                Ok(success) => {
                    let mut success = success;
                    let capacity_permit = permit;
                    success.stream = Box::pin(success.stream.map(move |item| {
                        let _keep_capacity_permit_alive = &capacity_permit;
                        item
                    }));
                    trace_builder
                        .success()
                        .channel(&candidate.channel_id, &candidate.channel_name)
                        .duration_ms(elapsed_ms(started))
                        .sticky(candidate.sticky)
                        .score(candidate.score)
                        .finish();
                    return RelayStreamRunOutcome {
                        response: Some(success),
                        selected_channel_id: Some(candidate.channel_id),
                        status_code: 200,
                        error_message: None,
                        attempts: trace_builder.finish_all(),
                    };
                }
                Err(error) => {
                    drop(permit);
                    trace_builder
                        .failed()
                        .channel(&candidate.channel_id, &candidate.channel_name)
                        .reason(error.message.clone())
                        .duration_ms(elapsed_ms(started))
                        .sticky(candidate.sticky)
                        .score(candidate.score)
                        .finish();

                    let proxy_error_for_feedback = error.proxy_error.as_ref().cloned().unwrap_or_else(|| {
                        ProxyError::UpstreamError {
                            status: axum::http::StatusCode::from_u16(error.status_code)
                                .unwrap_or(axum::http::StatusCode::BAD_GATEWAY),
                            body: error.message.clone(),
                        }
                    });
                    self.executor
                        .on_attempt_failed(&candidate.channel_id, &proxy_error_for_feedback)
                        .await;

                    last_error = Some(error);
                    if result.response_written {
                        break;
                    }
                }
            }
        }

        let (status_code, error_message) = last_error
            .map(|e| (e.status_code, Some(e.message)))
            .unwrap_or((503, Some("no available candidate".to_string())));

        RelayStreamRunOutcome {
            response: None,
            selected_channel_id: None,
            status_code,
            error_message,
            attempts: trace_builder.finish_all(),
        }
    }
}

pub struct RelayStreamRunOutcome {
    pub response: Option<RelayStreamSuccess>,
    pub selected_channel_id: Option<String>,
    pub status_code: u16,
    pub error_message: Option<String>,
    #[allow(dead_code)]
    pub attempts: Vec<AttemptTrace>,
}