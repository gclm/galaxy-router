use async_trait::async_trait;
use std::sync::{Arc, Mutex};

use crate::api::handlers::admin::channels::EndpointType;
use crate::metrics::attempt::AttemptStats;
use crate::metrics::usage::{calculate_cost, resolve_non_stream_usage};
use crate::relay::converter::RelayPipeline;
use crate::relay::prepare::{estimate_request_input_tokens, failed_attempt_stats, prepare_proxy_request};
use crate::relay::run::{
    RelayAttemptError, RelayAttemptExecutor, RelayAttemptResult, RelayCandidate, RelayRequest,
};
use crate::error::proxy::ProxyError;
use crate::relay::state::{ProxyState, ProxySuccess};
use crate::scheduler::selector::SelectionResult;

/// RAII guard：确保函数退出时自动递减活跃请求数
struct ActiveRequestGuard {
    state: ProxyState,
    channel_id: String,
}

impl Drop for ActiveRequestGuard {
    fn drop(&mut self) {
        let state = self.state.clone();
        let channel_id = self.channel_id.clone();
        tokio::spawn(async move {
            state.lb_state.decrement_active(&channel_id).await;
        });
    }
}

/// 非流式代理执行器：将 RelayRun 的候选迭代与真实 proxy 执行连接
#[derive(Clone)]
pub(crate) struct ProxyRelayExecutor {
    state: ProxyState,
    headers: axum::http::HeaderMap,
    body: serde_json::Value,
    client_endpoint: EndpointType,
    #[allow(dead_code)]
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
}

#[async_trait]
impl RelayAttemptExecutor for ProxyRelayExecutor {
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

        // 503 同渠道退避重试（参考 sub2api HandleSelectionExhausted 模式）：
        // 所有真正执行的 key 都返回 503 → 上游瞬时过载，共享资源问题，
        // 换 key 无效。等 2s 后清掉 per-key 熔断器再试同一批 key。
        let first_pass = self.run_key_loop(&selection, candidate).await;
        if first_pass.should_retry_503() {
            tracing::warn!(
                "所有 key 返回 503，2s 后同渠道重试: channel={}",
                candidate.channel_id
            );
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            self.state
                .lb_state
                .circuit_breaker
                .reset_channel(&candidate.channel_id)
                .await;
            return self.run_key_loop(&selection, candidate).await.into_result();
        }
        first_pass.into_result()
    }
}

impl ProxyRelayExecutor {
    /// 单轮遍历所有 key，执行代理请求。
    ///
    /// 返回 KeyLoopOutcome 让上层判断是否要做"503 同渠道退避重试"。
    async fn run_key_loop(
        &self,
        selection: &SelectionResult,
        candidate: &RelayCandidate,
    ) -> KeyLoopOutcome {
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

            let mut local_attempts = Vec::new();

            let result = self
                .execute_proxy_request(
                    upstream_api_key,
                    &key_hint,
                    selection,
                    &mut local_attempts,
                )
                .await;

            if let Ok(mut stats) = self.attempt_stats.lock() {
                stats.extend(local_attempts);
            }

            match result {
                Ok(success) => {
                    return KeyLoopOutcome::Success(
                        String::from_utf8_lossy(&success.body).to_string(),
                    );
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
                        // per-key 熔断：记录该 key 失败，连续失败会熔断此 key（不影响其他 key）
                        self.state
                            .lb_state
                            .circuit_breaker
                            .record_failure(&candidate.channel_id, &key_hint)
                            .await;
                        tracing::warn!(
                            "key retry: channel={}, status={}, trying next key",
                            candidate.channel_id,
                            status
                        );
                        last_error = Some(error);
                        continue;
                    }

                    return KeyLoopOutcome::NonKeyRetryableError(error);
                }
                Err(e) => {
                    // 仅 UpstreamError 走 key-retryable 路径（上方分支）。
                    // 其余 ProxyError 变体（DatabaseError / RequestError / TransformError 等）
                    // 非上游错误，不应触发换 key，直接 NonKeyRetryableError。
                    return KeyLoopOutcome::NonKeyRetryableError(
                        RelayAttemptError::from_proxy_error(e),
                    );
                }
            }
        }

        KeyLoopOutcome::AllKeysTried {
            last_error,
            all_executed_503: executed_count > 0 && all_executed_503,
        }
    }
}

/// run_key_loop 的返回结果。把"成功 / 非 key-retryable 失败 / 所有 key 已尝试"
/// 三种情况显式区分开，方便上层决策是否做 503 同渠道退避重试。
enum KeyLoopOutcome {
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
    fn should_retry_503(&self) -> bool {
        matches!(
            self,
            KeyLoopOutcome::AllKeysTried {
                all_executed_503: true,
                ..
            }
        )
    }

    /// 转换为 RelayAttemptResult
    fn into_result(self) -> RelayAttemptResult {
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

impl ProxyRelayExecutor {
    /// 执行单次非流式代理请求（从 proxy/execute.rs 内迁）
    async fn execute_proxy_request(
        &self,
        upstream_api_key: &str,
        upstream_key_hint: &str,
        selection: &SelectionResult,
        attempts: &mut Vec<AttemptStats>,
    ) -> Result<ProxySuccess, ProxyError> {
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
        let _guard = ActiveRequestGuard {
            state: self.state.clone(),
            channel_id: channel_id.clone(),
        };

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
                return Err(ProxyError::RequestError(e.to_string()));
            }
        };

        let latency_ms = start_time.elapsed().as_millis() as i64;
        let status = response.status();
        let response_body = response.text().await.unwrap_or_default();

        let body_value: serde_json::Value =
            serde_json::from_str(&response_body).unwrap_or_default();
        let status_u16 = status.as_u16();

        let usage = resolve_non_stream_usage(
            &self.body,
            &body_value,
            &prepared.upstream_endpoint,
            status_u16,
        );
        let cost = calculate_cost(&self.state.model_registry, &prepared.target_model, usage).await;
        // 失败请求若上游未返回 usage，按请求体估算 input（output 保持 0，未生成）
        let input_tokens = if !status.is_success() && usage.input_tokens == 0 {
            estimate_request_input_tokens(&self.body, &prepared.target_model)
        } else {
            usage.input_tokens
        };
        let output_tokens = usage.output_tokens;
        let cache_read = usage.cache_read;
        let cache_creation = usage.cache_creation;

        attempts.push(AttemptStats {
            channel_id: prepared.channel_id.clone(),
            target_model: prepared.target_model.clone(),
            upstream_endpoint: prepared.upstream_endpoint.clone(),
            needs_conversion: prepared.needs_conversion,
            latency_ms,
            status_code: status_u16,
            input_tokens,
            output_tokens,
            cache_read,
            cache_creation,
            cost,
            error_message: if !status.is_success() {
                Some(response_body[..response_body.len().min(500)].to_string())
            } else {
                None
            },
            upstream_key_hint: upstream_key_hint.to_string(),
        });

        if !status.is_success() {
            tracing::warn!(
                "Upstream error: channel={}, status={}, body={}",
                prepared.channel_id,
                status,
                &response_body[..response_body.len().min(300)]
            );
            return Err(ProxyError::UpstreamError {
                status,
                body: response_body,
            });
        }

        self.state
            .lb_state
            .record_success(&prepared.channel_id, latency_ms as f64)
            .await;
        // per-key 熔断：成功重置该 key 的熔断状态
        self.state
            .lb_state
            .circuit_breaker
            .record_success(&prepared.channel_id, upstream_key_hint)
            .await;

        let final_body = if prepared.needs_conversion {
            let finalized = RelayPipeline::finalize_response_async(
                self.client_endpoint.clone(),
                prepared.upstream_endpoint.clone(),
                body_value,
                status.as_u16(),
            )
            .await
            .map_err(|e| ProxyError::TransformError(e.to_string()))?;
            serde_json::to_vec(&finalized.body).unwrap_or_default()
        } else {
            response_body.into_bytes()
        };

        Ok(ProxySuccess {
            status,
            body: final_body,
        })
    }
}

#[cfg(test)]
mod tests {
    // ============================================================
    // 端到端：mock 本地 upstream + 真实 ProxyState 调 proxy_request
    // ============================================================

    use crate::db::Database;
    use crate::metrics::model::ModelRegistry;
    use crate::error::proxy::ProxyError;
    use crate::relay::pipeline::proxy_request;
    use crate::relay::state::ProxyState;
    use axum::{Router, routing::post};

    async fn spawn_mock_upstream() -> String {
        use axum::extract::Json as AxJson;

        async fn mock_chat(AxJson(_body): AxJson<serde_json::Value>) -> AxJson<serde_json::Value> {
            AxJson(serde_json::json!({
                "id": "chatcmpl-test",
                "object": "chat.completion",
                "model": "gpt-4o",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "hi back"},
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 10,
                    "completion_tokens": 5,
                    "total_tokens": 15
                }
            }))
        }

        let app = Router::new().route("/v1/chat/completions", post(mock_chat));
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

    async fn make_state_with_channel(upstream_url: &str) -> (ProxyState, sqlx::SqlitePool) {
        let db_path = format!("/tmp/galaxy_execute_{}.db", uuid::Uuid::now_v7());
        let _ = std::fs::remove_file(&db_path);
        let db_url = format!("sqlite:{}?mode=rwc", db_path);
        let db = Database::new(&db_url).await.unwrap();
        let pool = db.pool().clone();

        let base_url = upstream_url.trim_end_matches("/chat/completions");
        let channel_id = "ch-mock";
        let api_keys = r#"[{"key":"sk-mock","note":"","enabled":true}]"#;
        let endpoints = format!(
            r#"[{{"type":"openai_chat","base_url":"{}","enabled":true}}]"#,
            base_url
        );
        let models = r#"["gpt-4o"]"#;
        sqlx::query(
            "INSERT INTO channels (id, name, api_keys, endpoints, models, enabled) \
             VALUES (?, 'mock', ?, ?, ?, 1)",
        )
        .bind(channel_id)
        .bind(api_keys)
        .bind(&endpoints)
        .bind(models)
        .execute(&pool)
        .await
        .unwrap();

        let group_id = "grp-mock";
        sqlx::query("INSERT INTO groups (id, name, enabled) VALUES (?, 'gpt-4o', 1)")
            .bind(group_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO group_items (id, group_id, channel_id, model_name) \
             VALUES ('item-mock', ?, ?, 'gpt-4o')",
        )
        .bind(group_id)
        .bind(channel_id)
        .execute(&pool)
        .await
        .unwrap();

        let registry = ModelRegistry::new(pool.clone());
        let state = ProxyState::new(pool.clone(), registry).await;
        (state, pool)
    }

    #[tokio::test]
    async fn proxy_request_passes_through_to_local_upstream() {
        use crate::api::handlers::admin::channels::EndpointType;

        let upstream = spawn_mock_upstream().await;
        let (state, pool) = make_state_with_channel(&upstream).await;

        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}]
        });
        let headers = axum::http::HeaderMap::new();
        let result = proxy_request(
            &state,
            "test-request-id",
            Some("key-1"),
            &headers,
            &body,
            &EndpointType::OpenAiChat,
        )
        .await
        .expect("proxy should succeed");

        assert_eq!(result.status, 200);
        let resp: serde_json::Value = serde_json::from_slice(&result.body).unwrap();
        assert_eq!(resp["choices"][0]["message"]["content"], "hi back");
        assert_eq!(resp["usage"]["prompt_tokens"], 10);

        let count: i32 = sqlx::query_scalar("SELECT COUNT(*) FROM usage_logs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1, "应记录 1 条请求日志");
    }

    #[tokio::test]
    async fn proxy_request_no_available_channel_returns_error() {
        use crate::api::handlers::admin::channels::EndpointType;

        let db_path = format!("/tmp/galaxy_execute_empty_{}.db", uuid::Uuid::now_v7());
        let _ = std::fs::remove_file(&db_path);
        let db_url = format!("sqlite:{}?mode=rwc", db_path);
        let db = Database::new(&db_url).await.unwrap();
        let pool = db.pool().clone();
        let registry = ModelRegistry::new(pool.clone());
        let state = ProxyState::new(pool.clone(), registry).await;

        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}]
        });
        let headers = axum::http::HeaderMap::new();
        let result = proxy_request(
            &state,
            "test-request-id",
            Some("key-1"),
            &headers,
            &body,
            &EndpointType::OpenAiChat,
        )
        .await;
        let err = match result {
            Ok(_) => panic!("expected NoAvailableChannel, got Ok"),
            Err(e) => e,
        };
        assert!(matches!(err, ProxyError::ModelNotFound(_)), "got {:?}", err);

        let count: i32 = sqlx::query_scalar("SELECT COUNT(*) FROM usage_logs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1, "渠道选择失败也应记录日志");
        let status: Option<i32> = sqlx::query_scalar("SELECT status_code FROM usage_logs LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(status, Some(404));
    }
}
