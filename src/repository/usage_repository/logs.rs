//! logs 查询：请求日志列表（分页+过滤）/ 详情（含 payload）/ 模型去重列表。

use sqlx::{AssertSqlSafe, QueryBuilder};

use super::{SqliteUsageRepository, UsageLogDetail, UsageLogRow};
use crate::domain::usage::{LogsFilter, PagedResult};

pub(super) async fn get_logs(
    repo: &SqliteUsageRepository,
    filter: LogsFilter,
) -> Result<PagedResult<UsageLogRow>, sqlx::Error> {
    let mut count_builder = QueryBuilder::new("SELECT COUNT(*) FROM usage_logs ul WHERE 1=1");
    if let Some(ref model) = filter.model {
        count_builder.push(" AND ul.requested_model = ");
        count_builder.push_bind(model.clone());
    }
    if let Some(ref cid) = filter.channel_id {
        count_builder.push(" AND ul.channel_id = ");
        count_builder.push_bind(cid.clone());
    }
    if let Some(ref kid) = filter.api_key_id {
        count_builder.push(" AND ul.api_key_id = ");
        count_builder.push_bind(kid.clone());
    }
    match filter.status.as_deref() {
        Some("success") => {
            count_builder.push(" AND ul.status_code >= 200 AND ul.status_code < 400");
        }
        Some("failure") => {
            count_builder.push(" AND (ul.status_code < 200 OR ul.status_code >= 400)");
        }
        _ => {}
    }

    let total: (i64,) = count_builder
        .build_query_as()
        .fetch_one(&repo.pool)
        .await?;

    let tz = repo.tz_modifier();
    let mut data_builder = QueryBuilder::new(format!(
        r#"SELECT ul.id, ul.api_key_id, ak.name as api_key_name,
                  ul.channel_id, c.name as channel_name,
                  ul.route_id, ul.requested_model, ul.actual_model,
                  ul.input_tokens, ul.output_tokens,
                  ul.cache_read_tokens, ul.cache_creation_tokens,
                  ul.cost, ul.latency_ms, ul.ttft_ms, ul.status_code, ul.error_message, datetime(ul.created_at, '{}') as created_at,
                  ul.endpoint_type, ul.request_type, ul.is_stream, ul.upstream_key_hint, ul.user_agent, ul.attempts as raw_attempts
           FROM usage_logs ul
           LEFT JOIN api_keys ak ON ul.api_key_id = ak.id
           LEFT JOIN channels c ON ul.channel_id = c.id
           WHERE 1=1"#,
        tz
    ));
    if let Some(ref model) = filter.model {
        data_builder.push(" AND ul.requested_model = ");
        data_builder.push_bind(model.clone());
    }
    if let Some(ref cid) = filter.channel_id {
        data_builder.push(" AND ul.channel_id = ");
        data_builder.push_bind(cid.clone());
    }
    if let Some(ref kid) = filter.api_key_id {
        data_builder.push(" AND ul.api_key_id = ");
        data_builder.push_bind(kid.clone());
    }
    match filter.status.as_deref() {
        Some("success") => {
            data_builder.push(" AND ul.status_code >= 200 AND ul.status_code < 400");
        }
        Some("failure") => {
            data_builder.push(" AND (ul.status_code < 200 OR ul.status_code >= 400)");
        }
        _ => {}
    }
    data_builder.push(" ORDER BY ul.created_at DESC LIMIT ");
    data_builder.push(filter.limit);
    data_builder.push(" OFFSET ");
    data_builder.push(filter.offset);

    let rows: Vec<UsageLogRow> = data_builder
        .build_query_as()
        .fetch_all(&repo.pool)
        .await?
        .into_iter()
        .map(|mut row: UsageLogRow| {
            row.attempts = row
                .raw_attempts
                .take()
                .and_then(|s| serde_json::from_str(&s).ok());
            row
        })
        .collect();

    Ok(PagedResult {
        items: rows,
        total: total.0,
    })
}

pub(super) async fn get_log_detail(
    repo: &SqliteUsageRepository,
    id: &str,
) -> Result<Option<UsageLogDetail>, sqlx::Error> {
    let tz = repo.tz_modifier();
    let row = sqlx::query_as::<_, UsageLogDetail>(
        AssertSqlSafe(format!(r#"SELECT ul.id, ul.api_key_id, ak.name as api_key_name,
                  ul.channel_id, c.name as channel_name,
                  ul.route_id, ul.requested_model, ul.actual_model,
                  ul.input_tokens, ul.output_tokens,
                  ul.cache_read_tokens, ul.cache_creation_tokens,
                  ul.cost, ul.latency_ms, ul.ttft_ms, ul.status_code, ul.error_message, datetime(ul.created_at, '{}') as created_at,
                  ul.endpoint_type, ul.request_type,
                  up.request_content, up.response_content, ul.is_stream, ul.upstream_key_hint, ul.user_agent, ul.attempts as raw_attempts
           FROM usage_logs ul
           LEFT JOIN usage_payloads up ON up.log_id = ul.id
           LEFT JOIN api_keys ak ON ul.api_key_id = ak.id
           LEFT JOIN channels c ON ul.channel_id = c.id
           WHERE ul.id = ?"#, tz).as_str()),
    )
    .bind(id)
    .fetch_optional(&repo.pool)
    .await?
    .map(|mut r| {
        r.attempts = r.raw_attempts.take().and_then(|s| serde_json::from_str(&s).ok());
        r
    });

    Ok(row)
}

pub(super) async fn get_log_models(repo: &SqliteUsageRepository) -> Result<Vec<String>, sqlx::Error> {
    let models = sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT requested_model FROM usage_logs ORDER BY requested_model",
    )
    .fetch_all(&repo.pool)
    .await?;

    Ok(models)
}
