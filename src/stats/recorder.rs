use crate::api::response::generate_id;
use sqlx::SqlitePool;

/// 统计记录器
#[derive(Clone)]
pub struct StatsRecorder {
    pool: SqlitePool,
}

/// 单次渠道尝试记录
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChannelAttempt {
    pub channel_id: String,
    pub channel_name: Option<String>,
    pub status: String,
    pub duration_ms: i64,
    pub error: Option<String>,
    pub upstream_key_hint: Option<String>,
}

/// 记录请求
#[derive(Debug)]
pub struct RequestRecord {
    pub api_key_id: Option<String>,
    pub channel_id: Option<String>,
    pub group_id: Option<String>,
    pub requested_model: String,
    pub actual_model: Option<String>,
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub cache_read_tokens: i32,
    pub cache_creation_tokens: i32,
    pub cost: Option<f64>,
    pub latency_ms: Option<i32>,
    pub ttft_ms: Option<i32>,
    pub status_code: Option<i32>,
    pub error_message: Option<String>,
    pub endpoint_type: Option<String>,
    pub request_type: String,
    pub request_content: Option<String>,
    pub response_content: Option<String>,
    pub is_stream: bool,
    pub upstream_key_hint: Option<String>,
    pub attempts: Vec<ChannelAttempt>,
    pub user_agent: Option<String>,
}

impl StatsRecorder {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// 记录请求日志
    pub async fn record_request(&self, record: RequestRecord) -> Result<(), sqlx::Error> {
        let id = generate_id();
        let attempts_json = if record.attempts.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&record.attempts).unwrap_or_default())
        };

        sqlx::query(
            r#"
            INSERT INTO usage_logs (
                id, api_key_id, channel_id, group_id,
                requested_model, actual_model,
                input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                cost, latency_ms, ttft_ms, status_code, error_message,
                endpoint_type, request_type, request_content, response_content, is_stream,
                upstream_key_hint, attempts, user_agent
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(&record.api_key_id)
        .bind(&record.channel_id)
        .bind(&record.group_id)
        .bind(&record.requested_model)
        .bind(&record.actual_model)
        .bind(record.input_tokens)
        .bind(record.output_tokens)
        .bind(record.cache_read_tokens)
        .bind(record.cache_creation_tokens)
        .bind(record.cost)
        .bind(record.latency_ms)
        .bind(record.ttft_ms)
        .bind(record.status_code)
        .bind(&record.error_message)
        .bind(&record.endpoint_type)
        .bind(&record.request_type)
        .bind(&record.request_content)
        .bind(&record.response_content)
        .bind(record.is_stream)
        .bind(&record.upstream_key_hint)
        .bind(&attempts_json)
        .bind(&record.user_agent)
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn make_pool() -> sqlx::SqlitePool {
        let db_path = format!("/tmp/galaxy_stats_recorder_{}.db", uuid::Uuid::now_v7());
        let _ = std::fs::remove_file(&db_path);
        let db_url = format!("sqlite:{}?mode=rwc", db_path);
        crate::db::Database::new(&db_url)
            .await
            .unwrap()
            .pool()
            .clone()
    }

    fn base_record(attempts: Vec<ChannelAttempt>) -> RequestRecord {
        RequestRecord {
            api_key_id: Some("k1".into()),
            channel_id: Some("c1".into()),
            group_id: None,
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
        let rec = StatsRecorder::new(pool.clone());
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
        let rec = StatsRecorder::new(pool.clone());
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
}
