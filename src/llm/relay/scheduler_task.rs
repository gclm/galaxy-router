use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;

use crate::llm::relay::ratelimit::RateLimiter;
use crate::llm::scheduler::state::LoadBalancerState;
use crate::repository::channel_repository::{ChannelRepository, SqliteChannelRepository};
use crate::repository::settings_repository::{SettingsRepository, SqliteSettingsRepository};
use crate::repository::usage_repository::{SqliteUsageRepository, UsageRepository};

/// 健康探测默认间隔（秒）
const DEFAULT_PROBE_INTERVAL_SECS: u64 = 300;

/// 健康探测超时（秒）
const PROBE_TIMEOUT_SECS: u64 = 30;

/// 定时任务调度器
pub struct Scheduler {
    lb_state: LoadBalancerState,
    rate_limiter: RateLimiter,
    pool: sqlx::SqlitePool,
}

impl Scheduler {
    pub fn new(
        lb_state: LoadBalancerState,
        rate_limiter: RateLimiter,
        pool: sqlx::SqlitePool,
    ) -> Self {
        Self {
            lb_state,
            rate_limiter,
            pool,
        }
    }

    /// 启动定时任务
    pub fn start(self: Arc<Self>) {
        let scheduler = self.clone();
        tokio::spawn(async move {
            scheduler.run_cleanup().await;
        });

        let scheduler = self.clone();
        tokio::spawn(async move {
            scheduler.run_health_probes().await;
        });

        let scheduler = self.clone();
        tokio::spawn(async move {
            scheduler.run_log_cleanup().await;
        });

        let scheduler = self.clone();
        tokio::spawn(async move {
            scheduler.run_payload_cleanup().await;
        });
    }

    /// 清理任务
    async fn run_cleanup(&self) {
        let mut interval = interval(Duration::from_secs(60));

        loop {
            interval.tick().await;

            self.lb_state.cleanup_expired_sessions().await;
            self.lb_state.cleanup_expired_blacklists().await;
            self.rate_limiter.cleanup().await;
            self.lb_state
                .circuit_breaker
                .cleanup_expired(Duration::from_secs(3600))
                .await;
        }
    }

    /// 定期探测所有启用的渠道健康状况
    async fn run_health_probes(&self) {
        let mut interval = interval(Duration::from_secs(DEFAULT_PROBE_INTERVAL_SECS));

        loop {
            interval.tick().await;

            if let Err(e) = self.probe_all_channels().await {
                tracing::warn!("健康探测执行失败: {}", e);
            }
        }
    }

    /// 探测所有启用的渠道
    async fn probe_all_channels(&self) -> Result<(), String> {
        // 查询所有启用的渠道（需要 api_keys 和 endpoints 来构造探测请求）
        let channels = SqliteChannelRepository::new(self.pool.clone(), 0)
            .list_enabled_minimal()
            .await
            .map_err(|e| format!("查询渠道失败: {}", e))?;

        if channels.is_empty() {
            return Ok(());
        }

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(PROBE_TIMEOUT_SECS))
            .no_proxy()
            .build()
            .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

        // 并发探测所有渠道（最多 10 个同时进行）
        let semaphore = Arc::new(tokio::sync::Semaphore::new(10));
        let mut handles = Vec::new();

        for (channel_id, api_keys_str, endpoints_str, models_str) in channels {
            let sem = semaphore.clone();
            let client = client.clone();
            let lb_state = self.lb_state.clone();

            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.ok();

                let result = probe_single_channel(
                    &client,
                    &channel_id,
                    &api_keys_str,
                    &endpoints_str,
                    &models_str,
                )
                .await;

                match result {
                    Ok(latency_ms) => {
                        tracing::debug!(
                            "健康探测成功: channel={}, latency={}ms",
                            channel_id,
                            latency_ms
                        );
                        lb_state.record_success(&channel_id, latency_ms).await;
                        lb_state.record_monitor_success(&channel_id);
                        lb_state
                            .circuit_breaker
                            .record_success(&channel_id, "health")
                            .await;
                    }
                    Err(e) => {
                        tracing::warn!("健康探测失败: channel={}, error={}", channel_id, e);
                        lb_state.record_failure(&channel_id, false).await;
                        lb_state.record_monitor_failure(&channel_id);
                        lb_state
                            .circuit_breaker
                            .record_failure(&channel_id, "health")
                            .await;
                    }
                }
            }));
        }

        // 等待所有探测完成
        for handle in handles {
            let _ = handle.await;
        }

        Ok(())
    }

    /// 清理超过 90 天的请求日志（每天执行一次）
    async fn run_log_cleanup(&self) {
        let mut interval = interval(Duration::from_secs(86400));

        loop {
            interval.tick().await;

            // 保留天数可配（settings usage.retention_days，默认 30 天）
            let days = SqliteSettingsRepository::new(self.pool.clone())
                .get("usage.retention_days")
                .await
                .ok()
                .flatten()
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(30);
            let result = SqliteUsageRepository::new(self.pool.clone(), 0)
                .delete_older_than(days)
                .await;

            match result {
                Ok(deleted) => {
                    if deleted > 0 {
                        tracing::info!("清理了 {} 条过期请求日志（>{}天）", deleted, days);
                    }
                }
                Err(e) => {
                    tracing::warn!("清理请求日志失败: {}", e);
                }
            }
        }
    }

    /// 定期清理过期 usage_payloads（请求/响应原文），保留 usage_logs 统计行（分层 retention）
    async fn run_payload_cleanup(&self) {
        let mut interval = interval(Duration::from_secs(86400));

        loop {
            interval.tick().await;

            // content 保留天数可配（settings usage.payload_retention_days，默认 7 天）
            let days = SqliteSettingsRepository::new(self.pool.clone())
                .get("usage.payload_retention_days")
                .await
                .ok()
                .flatten()
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(7);
            let result = SqliteUsageRepository::new(self.pool.clone(), 0)
                .delete_payloads_older_than(days)
                .await;

            match result {
                Ok(deleted) => {
                    if deleted > 0 {
                        tracing::info!(
                            "清理了 {} 条过期 payload（>{}天，保留 usage_logs 统计）",
                            deleted,
                            days
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!("清理 payload 失败: {}", e);
                }
            }
        }
    }
}

/// 探测单个渠道：发送最小化请求并检查响应
async fn probe_single_channel(
    client: &reqwest::Client,
    _channel_id: &str,
    api_keys_str: &str,
    endpoints_str: &str,
    models_str: &str,
) -> Result<f64, String> {
    // 解析 API keys
    let api_keys: Vec<crate::domain::channel::UpstreamApiKey> =
        crate::domain::channel::parse_api_keys(api_keys_str);
    let api_key = api_keys
        .iter()
        .find(|k| k.enabled)
        .ok_or_else(|| "无可用 API Key".to_string())?;

    // 解析 endpoints
    let endpoints: Vec<crate::domain::channel::EndpointConfig> =
        serde_json::from_str(endpoints_str).unwrap_or_default();
    let endpoint = endpoints
        .iter()
        .find(|e| e.enabled)
        .ok_or_else(|| "无可用端点".to_string())?;

    // 解析模型列表，取第一个模型用于探测
    let models: Vec<String> = serde_json::from_str(models_str).unwrap_or_default();
    let model = models.first().map(|m| m.as_str()).unwrap_or("gpt-4o-mini");

    // 构造最小化探测请求
    let (url, body, auth_header) = build_probe_request(
        &endpoint.base_url,
        &endpoint.endpoint_type,
        &api_key.key,
        model,
    );

    let start = std::time::Instant::now();
    let mut req_builder = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Authorization", &auth_header);

    // Anthropic 兼容
    if matches!(
        endpoint.endpoint_type,
        crate::domain::channel::EndpointType::Anthropic
    ) {
        req_builder = req_builder
            .header("x-api-key", api_key.key.as_str())
            .header("anthropic-version", "2023-06-01");
    }

    let resp = req_builder
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let latency_ms = start.elapsed().as_millis() as f64;
    let status = resp.status();

    if status.is_success() {
        // 消费响应体以释放连接
        let _ = resp.bytes().await;
        Ok(latency_ms)
    } else {
        let text = resp.text().await.unwrap_or_default();
        Err(format!("HTTP {}: {}", status, &text[..text.len().min(200)]))
    }
}

/// 构造最小化探测请求（使用 max_tokens=1 降低成本）
fn build_probe_request(
    base_url: &str,
    endpoint_type: &crate::domain::channel::EndpointType,
    api_key: &str,
    model: &str,
) -> (String, serde_json::Value, String) {
    use crate::domain::channel::EndpointType;

    let (path, body) = match endpoint_type {
        EndpointType::OpenAiChat => (
            "/chat/completions",
            serde_json::json!({
                "model": model,
                "messages": [{"role": "user", "content": "hi"}],
                "max_tokens": 1,
                "stream": false
            }),
        ),
        EndpointType::OpenAiResponse => (
            "/responses",
            serde_json::json!({
                "model": model,
                "input": "hi",
                "max_output_tokens": 1
            }),
        ),
        EndpointType::Anthropic => (
            "/messages",
            serde_json::json!({
                "model": model,
                "messages": [{"role": "user", "content": "hi"}],
                "max_tokens": 1
            }),
        ),
        _ => (
            "/chat/completions",
            serde_json::json!({
                "model": model,
                "messages": [{"role": "user", "content": "hi"}],
                "max_tokens": 1,
                "stream": false
            }),
        ),
    };

    let url = format!("{}{}", base_url.trim_end_matches('/'), path);
    let auth = format!("Bearer {}", api_key);

    (url, body, auth)
}
