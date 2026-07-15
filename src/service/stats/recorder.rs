use std::sync::Arc;

use super::redaction;
use crate::error::proxy::ProxyError;
use crate::app_state::AppState;
use crate::service::stats::attempt::AttemptStats;
use crate::relay::ratelimit::RateLimiter;
use crate::repository::settings_repository::SettingsRepository;
use crate::repository::usage_repository::UsageRepository;

/// 统计记录器（service 层）：持 usage + settings repository，编排 redaction + 落库。
/// 落库 SQL 下沉 repository（insert_usage_log/insert_usage_payload，B2-C1），本层只做 redaction + 开关。
#[derive(Clone)]
pub struct StatsRecorder {
    pub usage: Arc<dyn UsageRepository>,
    pub settings: Arc<dyn SettingsRepository>,
}

// ChannelAttempt + RequestRecord 已归 domain/usage（B2-C0，持久化领域模型），此处 re-export
// 保兼容；impl RequestRecord 块（from_last_attempt 等）留本模块（service 层 impl 本 crate 类型）。
pub use crate::domain::usage::{ChannelAttempt, RequestRecord};

impl StatsRecorder {
    pub fn new(usage: Arc<dyn UsageRepository>, settings: Arc<dyn SettingsRepository>) -> Self {
        Self { usage, settings }
    }

    /// 记录请求日志：redaction（service）→ insert_usage_log（repo）→ settings 开关
    /// （repo）→ 条件 insert_usage_payload（repo）。落库 SQL 下沉 repository（B2-C1）。
    pub async fn record_request(&self, record: RequestRecord) -> Result<(), sqlx::Error> {
        // redaction 统一（service 层职责，承接所有 9 处调用点）
        let req_sanitized = record
            .request_content
            .as_deref()
            .map(redaction::sanitize_json_content);
        let resp_sanitized = record
            .response_content
            .as_deref()
            .map(redaction::sanitize_json_content);

        // usage_logs 统计字段（repository 生成 PK + 序列化 attempts + INSERT）
        let id = self.usage.insert_usage_log(&record).await?;

        // usage_payloads：受 usage.record_content 开关控制（默认 true），仅在有原文时写
        let record_content = self
            .settings
            .get("usage.record_content")
            .await?
            .and_then(|v| v.parse::<bool>().ok())
            .unwrap_or(true);
        if record_content && (req_sanitized.is_some() || resp_sanitized.is_some()) {
            self.usage
                .insert_usage_payload(&id, req_sanitized.as_deref(), resp_sanitized.as_deref())
                .await?;
        }

        Ok(())
    }
}

impl RequestRecord {
    /// 从最后一次尝试构造完整记录
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_last_attempt(
        last: &AttemptStats,
        request_id: Option<String>,
        api_key_id: Option<&str>,
        route_id: Option<&str>,
        model: &str,
        request_content: Option<String>,
        response_content: Option<String>,
        channel_attempts: Vec<ChannelAttempt>,
        ttft_ms: Option<i32>,
        is_stream: bool,
        user_agent: Option<String>,
    ) -> Self {
        Self {
            request_id,
            api_key_id: api_key_id.map(str::to_string),
            channel_id: Some(last.channel_id.clone()),
            route_id: route_id.map(str::to_string),
            requested_model: model.to_string(),
            actual_model: Some(last.target_model.clone()),
            input_tokens: last.input_tokens,
            output_tokens: last.output_tokens,
            cache_read_tokens: last.cache_read,
            cache_creation_tokens: last.cache_creation,
            cost: last.cost,
            latency_ms: Some(last.latency_ms as i32),
            ttft_ms,
            status_code: Some(last.status_code as i32),
            error_message: last.error_message.clone(),
            endpoint_type: Some(last.upstream_endpoint.as_str().to_string()),
            request_type: if last.needs_conversion {
                "conversion".to_string()
            } else {
                "passthrough".to_string()
            },
            request_content,
            response_content,
            is_stream,
            upstream_key_hint: Some(last.upstream_key_hint.clone()),
            attempts: channel_attempts,
            user_agent,
        }
    }

    /// 构造选择阶段失败时的最小记录（503 + "请求未到达上游"）
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn minimal_for_select_failure(
        request_id: Option<String>,
        api_key_id: Option<&str>,
        route_id: Option<&str>,
        model: &str,
        request_content: Option<String>,
        response_content: Option<String>,
        channel_attempts: Vec<ChannelAttempt>,
        is_stream: bool,
        user_agent: Option<String>,
        status_code: i32,
        error_message: &str,
    ) -> Self {
        Self {
            request_id,
            api_key_id: api_key_id.map(str::to_string),
            channel_id: None,
            route_id: route_id.map(str::to_string),
            requested_model: model.to_string(),
            actual_model: None,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            cost: None,
            latency_ms: None,
            ttft_ms: None,
            status_code: Some(status_code),
            error_message: Some(error_message.to_string()),
            endpoint_type: None,
            request_type: "unknown".to_string(),
            request_content,
            response_content,
            is_stream,
            upstream_key_hint: None,
            attempts: channel_attempts,
            user_agent,
        }
    }
}

pub(crate) fn channel_attempts_snapshot(attempts: &[AttemptStats]) -> Vec<ChannelAttempt> {
    attempts
        .iter()
        .map(|a| ChannelAttempt {
            channel_id: a.channel_id.clone(),
            channel_name: None,
            status: if (200..400).contains(&a.status_code) {
                "success".to_string()
            } else {
                "failed".to_string()
            },
            duration_ms: a.latency_ms,
            error: a.error_message.clone(),
            upstream_key_hint: Some(a.upstream_key_hint.clone()),
        })
        .collect()
}

/// 保存单条请求日志（汇总所有尝试）
#[allow(clippy::too_many_arguments)]
pub(crate) async fn save_request_record(
    state: &AppState,
    request_id: Option<String>,
    api_key_id: Option<&str>,
    route_id: Option<&str>,
    model: &str,
    request_content: Option<String>,
    response_content: Option<String>,
    attempts: &[AttemptStats],
    ttft_ms: Option<i32>,
    is_stream: bool,
    user_agent: Option<String>,
    select_error: Option<&ProxyError>,
) {
    let channel_attempts = channel_attempts_snapshot(attempts);

    let record = match attempts.last() {
        Some(last) => RequestRecord::from_last_attempt(
            last,
            request_id,
            api_key_id,
            route_id,
            model,
            request_content,
            response_content,
            channel_attempts,
            ttft_ms,
            is_stream,
            user_agent,
        ),
        None => {
            let (status, msg) = select_error
                .map(|e| match e {
                    ProxyError::ModelNotFound(m) => (404, m.clone()),
                    ProxyError::NoAvailableChannel(m) => (503, m.clone()),
                    ProxyError::ModelNotSupported(m) => (400, m.clone()),
                    ProxyError::DatabaseError(m) => (500, m.clone()),
                    ProxyError::ChannelNotFound(m) => (404, m.clone()),
                    ProxyError::RequestError(m) => (502, m.clone()),
                    ProxyError::TransformError(m) => (500, m.clone()),
                    ProxyError::UpstreamError { status, body } => {
                        (status.as_u16() as i32, format!("上游错误: {} {}", status, &body[..body.len().min(500)]))
                    }
                })
                .unwrap_or((503, "请求未到达上游".to_string()));
            RequestRecord::minimal_for_select_failure(
                request_id,
                api_key_id,
                route_id,
                model,
                request_content,
                response_content,
                channel_attempts,
                is_stream,
                user_agent,
                status,
                &msg,
            )
        }
    };

    if let Err(e) = state.stats_recorder.record_request(record).await {
        tracing::error!("Failed to save usage log: {e}");
    }
    record_rate_limit_tokens(&state.rate_limiter, api_key_id, attempts).await;
}

async fn record_rate_limit_tokens(
    rate_limiter: &RateLimiter,
    api_key_id: Option<&str>,
    attempts: &[AttemptStats],
) {
    if let Some(key_id) = api_key_id {
        let total_input: i32 = attempts.iter().map(|a| a.input_tokens).sum();
        let total_output: i32 = attempts.iter().map(|a| a.output_tokens).sum();
        if total_input > 0 || total_output > 0 {
            rate_limiter
                .record_tokens(key_id, total_input as u64, total_output as u64)
                .await;
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn record_stream_completion(
    stats_recorder: StatsRecorder,
    rate_limiter: RateLimiter,
    request_id: String,
    api_key_id: Option<String>,
    channel_id: String,
    route_id: Option<String>,
    requested_model: String,
    actual_model: String,
    input_tokens: i32,
    output_tokens: i32,
    cache_read: i32,
    cache_creation: i32,
    cost: Option<f64>,
    latency_ms: i64,
    ttft_ms: Option<i32>,
    status_code: i32,
    error_message: Option<String>,
    endpoint_type: String,
    needs_conversion: bool,
    request_content: Option<String>,
    response_content: Option<String>,
    upstream_key_hint: String,
    mut channel_attempts: Vec<ChannelAttempt>,
    user_agent: Option<String>,
) {
    channel_attempts.push(ChannelAttempt {
        channel_id: channel_id.clone(),
        channel_name: None,
        status: if (200..400).contains(&status_code) {
            "success".into()
        } else {
            "failed".into()
        },
        duration_ms: latency_ms,
        error: error_message.clone(),
        upstream_key_hint: Some(upstream_key_hint.clone()),
    });

    let rate_limit_key = api_key_id.clone();
    let record = RequestRecord {
        request_id: Some(request_id),
        api_key_id,
        channel_id: Some(channel_id),
        route_id,
        requested_model,
        actual_model: Some(actual_model),
        input_tokens,
        output_tokens,
        cache_read_tokens: cache_read,
        cache_creation_tokens: cache_creation,
        cost,
        latency_ms: Some(latency_ms as i32),
        ttft_ms,
        status_code: Some(status_code),
        error_message,
        endpoint_type: Some(endpoint_type),
        request_type: if needs_conversion {
            "conversion".into()
        } else {
            "passthrough".into()
        },
        request_content,
        response_content,
        is_stream: true,
        upstream_key_hint: Some(upstream_key_hint),
        attempts: channel_attempts,
        user_agent,
    };

    // DB 写入放到独立 spawn：脱离调用方 context，避免客户端断开导致 usage_logs 漏记。
    // 10s timeout 是最后兜底，防止 DB 卡死把 cleanup task 也拖住。
    match stats_recorder.record_request(record).await {
        Ok(()) => {
            if let Some(ref key_id) = rate_limit_key
                && (input_tokens > 0 || output_tokens > 0)
            {
                rate_limiter
                    .record_tokens(key_id, input_tokens as u64, output_tokens as u64)
                    .await;
            }
        }
        Err(e) => tracing::warn!("Failed to save stream stats: {}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_recorder(pool: &sqlx::SqlitePool) -> StatsRecorder {
        use crate::repository::settings_repository::SqliteSettingsRepository;
        use crate::repository::usage_repository::SqliteUsageRepository;
        StatsRecorder::new(
            Arc::new(SqliteUsageRepository::new(pool.clone(), 0)),
            Arc::new(SqliteSettingsRepository::new(pool.clone())),
        )
    }

    async fn make_pool() -> sqlx::SqlitePool {
        let db_path = format!("/tmp/galaxy_stats_recorder_{}.db", uuid::Uuid::now_v7());
        let _ = std::fs::remove_file(&db_path);
        let db_url = format!("sqlite:{}?mode=rwc", db_path);
        crate::infra::db::Database::new(&db_url)
            .await
            .unwrap()
            .pool()
            .clone()
    }

    fn base_record(attempts: Vec<ChannelAttempt>) -> RequestRecord {
        RequestRecord {
            request_id: None,
            api_key_id: Some("k1".into()),
            channel_id: Some("c1".into()),
            route_id: None,
            requested_model: "gpt-4o".into(),
            actual_model: None,
            input_tokens: 10,
            output_tokens: 5,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            cost: Some(0.0001),
            latency_ms: Some(120),
            ttft_ms: Some(50),
            status_code: Some(200),
            error_message: None,
            endpoint_type: Some("openai_chat".into()),
            request_type: "passthrough".into(),
            request_content: Some(r#"{"m":"x"}"#.into()),
            response_content: Some(r#"{"ok":true}"#.into()),
            is_stream: false,
            upstream_key_hint: Some("sk-...abc".into()),
            attempts,
            user_agent: Some("test/1.0".into()),
        }
    }

    #[tokio::test]
    async fn record_request_with_no_attempts_inserts_null() {
        let pool = make_pool().await;
        let rec = test_recorder(&pool);
        rec.record_request(base_record(vec![])).await.unwrap();

        let attempts: Option<String> =
            sqlx::query_scalar("SELECT attempts FROM usage_logs LIMIT 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(attempts.is_none());
    }

    #[tokio::test]
    async fn record_request_with_attempts_serializes_json() {
        let pool = make_pool().await;
        let rec = test_recorder(&pool);
        let attempts = vec![
            ChannelAttempt {
                channel_id: "c1".into(),
                channel_name: Some("OpenAI".into()),
                status: "ok".into(),
                duration_ms: 12,
                error: None,
                upstream_key_hint: Some("sk-...abc".into()),
            },
            ChannelAttempt {
                channel_id: "c2".into(),
                channel_name: None,
                status: "fail".into(),
                duration_ms: 5,
                error: Some("timeout".into()),
                upstream_key_hint: None,
            },
        ];
        rec.record_request(base_record(attempts)).await.unwrap();

        let attempts_json: String = sqlx::query_scalar("SELECT attempts FROM usage_logs LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: Vec<ChannelAttempt> = serde_json::from_str(&attempts_json).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].channel_id, "c1");
        assert_eq!(parsed[1].error.as_deref(), Some("timeout"));
    }

    fn sample_attempt(status_code: u16) -> AttemptStats {
        AttemptStats {
            channel_id: "ch-1".into(),
            target_model: "gpt-4o".into(),
            upstream_endpoint: crate::api::handlers::admin::channels::EndpointType::OpenAiChat,
            needs_conversion: false,
            latency_ms: 123,
            status_code,
            input_tokens: 10,
            output_tokens: 20,
            cache_read: 3,
            cache_creation: 0,
            cost: Some(0.001),
            error_message: None,
            upstream_key_hint: "sk-abcde...mnop".into(),
        }
    }

    fn sample_attempts() -> Vec<ChannelAttempt> {
        vec![ChannelAttempt {
            channel_id: "ch-1".into(),
            channel_name: None,
            status: "failed".into(),
            duration_ms: 100,
            error: Some("upstream 500".into()),
            upstream_key_hint: Some("sk-abcde...mnop".into()),
        }]
    }

    #[test]
    fn from_last_attempt_propagates_all_fields() {
        let last = sample_attempt(200);
        let record = RequestRecord::from_last_attempt(
            &last,
            Some("req-1".into()),
            Some("key-1"),
            Some("grp-1"),
            "gpt-4o",
            Some("hi".into()),
            Some("hello".into()),
            sample_attempts(),
            Some(45),
            false,
            Some("ua/1.0".into()),
        );
        assert_eq!(record.api_key_id.as_deref(), Some("key-1"));
        assert_eq!(record.channel_id.as_deref(), Some("ch-1"));
        assert_eq!(record.route_id.as_deref(), Some("grp-1"));
        assert_eq!(record.requested_model, "gpt-4o");
        assert_eq!(record.actual_model.as_deref(), Some("gpt-4o"));
        assert_eq!(record.input_tokens, 10);
        assert_eq!(record.output_tokens, 20);
        assert_eq!(record.cache_read_tokens, 3);
        assert_eq!(record.cost, Some(0.001));
        assert_eq!(record.latency_ms, Some(123));
        assert_eq!(record.ttft_ms, Some(45));
        assert_eq!(record.status_code, Some(200));
        assert_eq!(record.error_message, None);
        assert_eq!(record.endpoint_type.as_deref(), Some("openai_chat"));
        assert_eq!(record.request_type, "passthrough");
        assert!(!record.is_stream);
        assert_eq!(record.upstream_key_hint.as_deref(), Some("sk-abcde...mnop"));
        assert_eq!(record.attempts.len(), 1);
        assert_eq!(record.user_agent.as_deref(), Some("ua/1.0"));
    }

    #[test]
    fn from_last_attempt_marks_conversion_path() {
        let mut last = sample_attempt(200);
        last.needs_conversion = true;
        let record = RequestRecord::from_last_attempt(
            &last,
            None,
            None,
            None,
            "claude-sonnet",
            None,
            None,
            vec![],
            None,
            false,
            None,
        );
        assert_eq!(record.request_type, "conversion");
    }

    #[test]
    fn from_last_attempt_marks_passthrough_path() {
        let last = sample_attempt(200);
        let record = RequestRecord::from_last_attempt(
            &last,
            None,
            None,
            None,
            "gpt-4o",
            None,
            None,
            vec![],
            None,
            true,
            None,
        );
        assert_eq!(record.request_type, "passthrough");
        assert!(record.is_stream);
    }

    #[test]
    fn from_last_attempt_preserves_error_message_on_failed_status() {
        let mut last = sample_attempt(503);
        last.error_message = Some("upstream timeout".into());
        let record = RequestRecord::from_last_attempt(
            &last,
            None,
            None,
            None,
            "gpt-4o",
            None,
            None,
            vec![],
            None,
            false,
            None,
        );
        assert_eq!(record.status_code, Some(503));
        assert_eq!(record.error_message.as_deref(), Some("upstream timeout"));
    }

    #[test]
    fn minimal_for_select_failure_fills_503_and_marker_text() {
        let record = RequestRecord::minimal_for_select_failure(
            None,
            Some("key-1"),
            Some("grp-1"),
            "gpt-4o",
            Some("hi".into()),
            None,
            sample_attempts(),
            false,
            Some("ua/1.0".into()),
            503,
            "请求未到达上游",
        );
        assert_eq!(record.status_code, Some(503));
        assert_eq!(record.error_message.as_deref(), Some("请求未到达上游"));
        assert!(record.channel_id.is_none());
        assert!(record.actual_model.is_none());
        assert_eq!(record.input_tokens, 0);
        assert_eq!(record.output_tokens, 0);
        assert!(record.cost.is_none());
        assert!(record.latency_ms.is_none());
        assert!(record.ttft_ms.is_none());
        assert!(record.endpoint_type.is_none());
        assert!(record.upstream_key_hint.is_none());
        assert_eq!(record.request_type, "unknown");
        assert_eq!(record.attempts.len(), 1);
    }

    #[tokio::test]
    async fn record_request_writes_payload_and_redacts() {
        let pool = make_pool().await;
        let rec = test_recorder(&pool);
        let mut record = base_record(vec![]);
        record.request_content =
            Some(r#"{"authorization":"sk-secret123","prompt":"hi"}"#.into());
        record.response_content = Some(r#"{"content":"answer","api_key":"sk-leak"}"#.into());
        rec.record_request(record).await.unwrap();

        // usage_logs 仍写（拆表后无 content 列，但统计字段在）
        let log_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM usage_logs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(log_count, 1);

        // usage_payloads 写 content，且过 redaction（密钥 → [REDACTED]）
        let (req, resp): (Option<String>, Option<String>) =
            sqlx::query_as("SELECT request_content, response_content FROM usage_payloads LIMIT 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        let req = req.expect("request_content 应填充");
        assert!(req.contains("[REDACTED]"), "authorization 应脱敏: {}", req);
        assert!(!req.contains("sk-secret123"), "密钥不应明文: {}", req);
        assert!(req.contains("prompt"), "非敏感字段保留: {}", req);
        let resp = resp.expect("response_content 应填充");
        assert!(resp.contains("[REDACTED]"), "response api_key 应脱敏: {}", resp);
        assert!(!resp.contains("sk-leak"), "密钥不应明文: {}", resp);
    }

    #[tokio::test]
    async fn record_request_skips_payload_when_record_content_disabled() {
        let pool = make_pool().await;
        // 关闭开关
        sqlx::query("UPDATE settings SET value='false' WHERE key='usage.record_content'")
            .execute(&pool)
            .await
            .unwrap();
        let rec = test_recorder(&pool);
        rec.record_request(base_record(vec![])).await.unwrap();

        // usage_logs 仍写
        let log_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM usage_logs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(log_count, 1);
        // usage_payloads 不写（开关关闭）
        let payload_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM usage_payloads")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(payload_count, 0);
    }
}
