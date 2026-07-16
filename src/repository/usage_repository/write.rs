//! 写入：usage_logs（统计字段，返回 UUID v7 log_id）+ usage_payloads（脱敏原文）。

use super::SqliteUsageRepository;
use crate::domain::usage::RequestRecord;

pub(super) async fn insert_usage_log(
    repo: &SqliteUsageRepository,
    record: &RequestRecord,
) -> Result<String, sqlx::Error> {
    let id = uuid::Uuid::now_v7().to_string();
    let attempts_json = if record.attempts.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&record.attempts).unwrap_or_default())
    };
    sqlx::query(
        r#"
        INSERT INTO usage_logs (
            id, request_id, api_key_id, channel_id, route_id,
            requested_model, actual_model,
            input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
            cost, latency_ms, ttft_ms, status_code, error_message,
            endpoint_type, request_type, is_stream,
            upstream_key_hint, attempts, user_agent
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(&record.request_id)
    .bind(&record.api_key_id)
    .bind(&record.channel_id)
    .bind(&record.route_id)
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
    .bind(record.is_stream)
    .bind(&record.upstream_key_hint)
    .bind(&attempts_json)
    .bind(&record.user_agent)
    .execute(&repo.pool)
    .await?;
    Ok(id)
}

pub(super) async fn insert_usage_payload(
    repo: &SqliteUsageRepository,
    log_id: &str,
    request_content: Option<&str>,
    response_content: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT OR IGNORE INTO usage_payloads (log_id, request_content, response_content) VALUES (?, ?, ?)",
    )
    .bind(log_id)
    .bind(request_content)
    .bind(response_content)
    .execute(&repo.pool)
    .await?;
    Ok(())
}
