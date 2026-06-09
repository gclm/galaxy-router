use async_trait::async_trait;
use std::sync::{Arc, Mutex};

use crate::api::handlers::admin::channels::EndpointType;
use crate::proxy::execute::AttemptStats;
use crate::proxy::selection::SelectionResult;
use crate::proxy::{ProxyError, ProxyState};
use crate::relay::http::execute_once;
use crate::relay::run::{
    RelayAttemptError, RelayAttemptExecutor, RelayAttemptResult, RelayCandidate, RelayRequest,
};

/// 非流式代理执行器：将 RelayRun 的候选迭代与真实 proxy 执行连接
#[derive(Clone)]
pub(crate) struct ProxyRelayExecutor {
    state: ProxyState,
    headers: axum::http::HeaderMap,
    body: serde_json::Value,
    client_endpoint: EndpointType,
    api_key_id: Option<String>,
    attempt_stats: Arc<Mutex<Vec<AttemptStats>>>,
}

impl ProxyRelayExecutor {
    pub(crate) fn new(
        state: ProxyState,
        headers: axum::http::HeaderMap,
        body: serde_json::Value,
        client_endpoint: EndpointType,
        api_key_id: Option<String>,
    ) -> Self {
        Self {
            state,
            headers,
            body,
            client_endpoint,
            api_key_id,
            attempt_stats: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// 取出 executor 执行期间积累的 AttemptStats
    pub(crate) fn take_attempt_stats(&self) -> Vec<AttemptStats> {
        let mut stats = self
            .attempt_stats
            .lock()
            .expect("attempt_stats mutex poisoned");
        std::mem::take(&mut *stats)
    }

    /// 从 RelayCandidate 构建 SelectionResult（用于兼容 execute_proxy_request）
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
impl RelayAttemptExecutor for ProxyRelayExecutor {
    async fn is_channel_available(&self, channel_id: &str) -> bool {
        self.state.lb_state.is_channel_available(channel_id).await
    }

    async fn on_attempt_failed(&self, channel_id: &str, _status_code: u16, is_server_error: bool) {
        self.state
            .lb_state
            .record_failure(channel_id, is_server_error)
            .await;
    }

    async fn execute(
        &self,
        _request: &RelayRequest,
        candidate: &RelayCandidate,
    ) -> RelayAttemptResult {
        let selection = match self.build_selection(candidate).await {
            Ok(s) => s,
            Err(e) => {
                return RelayAttemptResult {
                    response: Err(e),
                    response_written: false,
                };
            }
        };

        // Key rotation：尝试同渠道的多个 API key
        let api_key_attempts = self.state.api_key_attempts(&selection.channel);
        let mut last_error = None;

        for upstream_api_key in &api_key_attempts {
            let key_hint = selection.channel.key_hint(upstream_api_key);
            let mut local_attempts = Vec::new();

            let result = execute_once(
                &self.state,
                self.api_key_id.as_deref(),
                upstream_api_key,
                &key_hint,
                &self.headers,
                &self.body,
                &self.client_endpoint,
                &selection,
                &mut local_attempts,
            )
            .await;

            // 将本次尝试的 AttemptStats 合并到 executor 的全局统计中
            if let Ok(mut stats) = self.attempt_stats.lock() {
                stats.extend(local_attempts);
            }

            match result {
                Ok(success) => {
                    // 成功：返回响应体（String）
                    return RelayAttemptResult {
                        response: Ok(String::from_utf8_lossy(&success.body).to_string()),
                        response_written: false,
                    };
                }
                Err(ProxyError::UpstreamError { status, body }) => {
                    let error = RelayAttemptError::new(status.as_u16(), sanitize_error_body(&body));
                    let is_key_retryable = ProxyError::UpstreamError {
                        status,
                        body: body.clone(),
                    }
                    .is_key_retryable();

                    if is_key_retryable {
                        tracing::warn!(
                            "key retry: channel={}, status={}, trying next key",
                            candidate.channel_id,
                            status
                        );
                        last_error = Some(error);
                        continue;
                    }

                    // 非 key 相关错误：返回失败（RelayRun 会尝试下一个候选）
                    return RelayAttemptResult {
                        response: Err(error),
                        response_written: false,
                    };
                }
                Err(e) => {
                    // 网络/内部错误
                    return RelayAttemptResult {
                        response: Err(RelayAttemptError::new(502, e.to_string())),
                        response_written: false,
                    };
                }
            }
        }

        // 所有 key 都失败了
        RelayAttemptResult {
            response: Err(
                last_error.unwrap_or_else(|| RelayAttemptError::new(500, "all api keys exhausted"))
            ),
            response_written: false,
        }
    }
}

/// 截断过长的错误消息
fn sanitize_error_body(body: &str) -> String {
    body[..body.len().min(500)].to_string()
}
