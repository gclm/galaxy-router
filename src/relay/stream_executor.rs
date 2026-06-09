use async_trait::async_trait;
use std::sync::{Arc, Mutex};

use crate::api::handlers::admin::channels::EndpointType;
use crate::metrics::attempt::AttemptStats;
use crate::proxy::{ProxyError, ProxyState};
use crate::relay::run::{
    RelayAttemptError, RelayCandidate, RelayRequest, RelayStreamAttemptExecutor,
    RelayStreamAttemptResult, RelayStreamSuccess,
};
use crate::relay::stream::execute_once;
use crate::scheduler::selector::SelectionResult;

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
            .find_endpoint(&self.client_endpoint)
            .ok_or_else(|| {
                RelayAttemptError::new(
                    503,
                    format!(
                        "channel {} has no endpoint for {}",
                        candidate.channel_id,
                        self.client_endpoint.as_str()
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

            let result = execute_once(
                &self.state,
                self.request_id.clone(),
                self.api_key_id.as_deref(),
                upstream_api_key,
                key_hint,
                selection.group_id.clone(),
                &self.headers,
                &self.body,
                &self.client_endpoint,
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
