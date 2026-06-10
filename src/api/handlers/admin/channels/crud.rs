use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use sqlx::{AssertSqlSafe, SqlitePool};

use super::types::{
    Channel, ChannelRow, ChannelState, CreateChannelRequest, ListChannelsQuery, PaginatedResponse,
    UpdateChannelRequest, UpstreamApiKey,
};
use crate::api::response::generate_id;
use crate::api::{ApiError, ApiResponse};
use crate::metrics::query::tz_modifier;

/// 获取渠道列表（支持搜索、筛选、排序、分页）
pub async fn list(
    State(state): State<ChannelState>,
    Query(query): Query<ListChannelsQuery>,
) -> Result<Json<ApiResponse<PaginatedResponse<Channel>>>, (StatusCode, Json<ApiError>)> {
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).clamp(1, 100);
    let offset = (page - 1) * page_size;

    let order_field = match query.sort_by.as_deref() {
        Some("name") => "name",
        _ => "created_at",
    };
    let order_dir = match query.sort_order.as_deref() {
        Some("asc") => "ASC",
        _ => "DESC",
    };

    let mut count_builder = sqlx::QueryBuilder::new("SELECT COUNT(*) FROM channels");
    let _has_where = push_where(&mut count_builder, &query);

    let count_row = count_builder
        .build()
        .fetch_one(&state.pool)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;
    let total: i64 = sqlx::Row::get(&count_row, 0);

    let tz = tz_modifier(state.timezone_offset);
    let mut data_builder = sqlx::QueryBuilder::new(format!(
        "SELECT id, name, api_keys, endpoints, models, rate_limit_rpm, rate_limit_tpm, failure_threshold, blacklist_minutes, concurrency, timeout_secs, max_concurrency, custom_headers, extras, enabled, datetime(created_at, '{}') as created_at, datetime(updated_at, '{}') as updated_at FROM channels",
        tz, tz
    ));
    push_where(&mut data_builder, &query);
    data_builder.push(format!(" ORDER BY {} {} ", order_field, order_dir));
    data_builder.push(" LIMIT ");
    data_builder.push_bind(page_size);
    data_builder.push(" OFFSET ");
    data_builder.push_bind(offset);

    let rows = data_builder
        .build()
        .fetch_all(&state.pool)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    let items: Vec<Channel> = rows
        .iter()
        .map(row_to_channel_from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(ApiError::internal_error)?;

    Ok(Json(ApiResponse::success(PaginatedResponse {
        items,
        total,
    })))
}

fn push_where(builder: &mut sqlx::QueryBuilder<sqlx::Sqlite>, query: &ListChannelsQuery) -> bool {
    let mut has_where = false;

    if let Some(ref search) = query.search
        && !search.is_empty()
    {
        builder.push(" WHERE name LIKE ");
        builder.push_bind(format!("%{}%", search));
        has_where = true;
    }
    if let Some(ref status) = query.status {
        let enabled_val = match status.as_str() {
            "enabled" => Some(true),
            "disabled" => Some(false),
            _ => None,
        };
        if let Some(v) = enabled_val {
            builder.push(if has_where {
                " AND enabled = "
            } else {
                " WHERE enabled = "
            });
            builder.push_bind(v);
            has_where = true;
        }
    }
    has_where
}

/// 创建渠道
pub async fn create(
    State(state): State<ChannelState>,
    Json(req): Json<CreateChannelRequest>,
) -> Result<(StatusCode, Json<ApiResponse<Channel>>), (StatusCode, Json<ApiError>)> {
    if req.name.is_empty() {
        return Err(ApiError::bad_request("渠道名称不能为空"));
    }
    if req.api_keys.is_empty() {
        return Err(ApiError::bad_request("至少需要一个 API Key"));
    }
    if req.endpoints.is_empty() {
        return Err(ApiError::bad_request("至少需要一个端点"));
    }
    for k in &req.api_keys {
        crate::relay::pipeline::validate_header_value(&k.key)
            .map_err(|e| ApiError::bad_request(format!("API Key 非法: {e}")))?;
    }

    let id = generate_id();
    let api_keys_json = serde_json::to_string(&req.api_keys)
        .map_err(|e| ApiError::internal_error(e.to_string()))?;
    let endpoints_json = serde_json::to_string(&req.endpoints)
        .map_err(|e| ApiError::internal_error(e.to_string()))?;
    let models_json = serde_json::to_string(&req.models.unwrap_or_default())
        .map_err(|e| ApiError::internal_error(e.to_string()))?;
    let custom_headers_json = serde_json::to_string(&req.custom_headers.unwrap_or_default())
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    sqlx::query(
        r#"
        INSERT INTO channels (id, name, api_keys, endpoints, models, rate_limit_rpm, rate_limit_tpm, failure_threshold, blacklist_minutes, concurrency, timeout_secs, max_concurrency, custom_headers, extras, enabled)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#
    )
    .bind(&id)
    .bind(&req.name)
    .bind(&api_keys_json)
    .bind(&endpoints_json)
    .bind(&models_json)
    .bind(req.rate_limit_rpm)
    .bind(req.rate_limit_tpm)
    .bind(req.failure_threshold.unwrap_or(3))
    .bind(req.blacklist_minutes.unwrap_or(10))
    .bind(req.concurrency.unwrap_or(10))
    .bind(req.timeout_secs.unwrap_or(300))
    .bind(req.max_concurrency.unwrap_or(0))
    .bind(&custom_headers_json)
    .bind(req.extras.as_ref().map(|m| serde_json::to_string(m).unwrap_or_else(|_| "{}".to_string())).unwrap_or_else(|| "{}".to_string()))
    .bind(req.enabled.unwrap_or(true))
    .execute(&state.pool)
    .await
    .map_err(|e| {
        if e.to_string().contains("UNIQUE constraint failed") {
            ApiError::conflict("渠道名称已存在")
        } else {
            ApiError::internal_error(e.to_string())
        }
    })?;

    let channel = get_channel_by_id(&state.pool, &id, state.timezone_offset).await?;
    state.cache.invalidate_all_channels().await;
    Ok((StatusCode::CREATED, Json(ApiResponse::success(channel))))
}

/// 获取单个渠道
pub async fn get(
    State(state): State<ChannelState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<Channel>>, (StatusCode, Json<ApiError>)> {
    let channel = get_channel_by_id(&state.pool, &id, state.timezone_offset).await?;
    Ok(Json(ApiResponse::success(channel)))
}

/// 更新渠道
pub async fn update(
    State(state): State<ChannelState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateChannelRequest>,
) -> Result<Json<ApiResponse<Channel>>, (StatusCode, Json<ApiError>)> {
    let existing = sqlx::query_scalar::<_, String>("SELECT id FROM channels WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.pool)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    if existing.is_none() {
        return Err(ApiError::not_found("渠道不存在"));
    }

    let mut builder = sqlx::QueryBuilder::new("UPDATE channels SET ");
    let mut separated = builder.separated(", ");
    let mut has_update = false;

    if let Some(ref name) = req.name {
        separated.push("name = ");
        separated.push_bind_unseparated(name);
        has_update = true;
    }
    if let Some(ref api_keys) = req.api_keys {
        for k in api_keys {
            crate::relay::pipeline::validate_header_value(&k.key)
                .map_err(|e| ApiError::bad_request(format!("API Key 非法: {e}")))?;
        }
        separated.push("api_keys = ");
        separated.push_bind_unseparated(serde_json::to_string(api_keys).unwrap_or_default());
        has_update = true;
    }
    if let Some(ref endpoints) = req.endpoints {
        separated.push("endpoints = ");
        separated.push_bind_unseparated(serde_json::to_string(endpoints).unwrap_or_default());
        has_update = true;
    }
    if let Some(ref models) = req.models {
        separated.push("models = ");
        separated.push_bind_unseparated(serde_json::to_string(models).unwrap_or_default());
        has_update = true;
    }
    if let Some(ref custom_headers) = req.custom_headers {
        separated.push("custom_headers = ");
        separated.push_bind_unseparated(serde_json::to_string(custom_headers).unwrap_or_default());
        has_update = true;
    }
    if let Some(ref extras) = req.extras {
        separated.push("extras = ");
        separated.push_bind_unseparated(
            serde_json::to_string(extras).unwrap_or_else(|_| "{}".to_string()),
        );
        has_update = true;
    }
    if let Some(enabled) = req.enabled {
        separated.push("enabled = ");
        separated.push_bind_unseparated(enabled);
        has_update = true;
    }
    if let Some(rate_limit_rpm) = req.rate_limit_rpm {
        separated.push("rate_limit_rpm = ");
        separated.push_bind_unseparated(rate_limit_rpm);
        has_update = true;
    }
    if let Some(rate_limit_tpm) = req.rate_limit_tpm {
        separated.push("rate_limit_tpm = ");
        separated.push_bind_unseparated(rate_limit_tpm);
        has_update = true;
    }
    if let Some(failure_threshold) = req.failure_threshold {
        separated.push("failure_threshold = ");
        separated.push_bind_unseparated(failure_threshold);
        has_update = true;
    }
    if let Some(blacklist_minutes) = req.blacklist_minutes {
        separated.push("blacklist_minutes = ");
        separated.push_bind_unseparated(blacklist_minutes);
        has_update = true;
    }
    if let Some(concurrency) = req.concurrency {
        separated.push("concurrency = ");
        separated.push_bind_unseparated(concurrency);
        has_update = true;
    }
    if let Some(timeout_secs) = req.timeout_secs {
        separated.push("timeout_secs = ");
        separated.push_bind_unseparated(timeout_secs);
        has_update = true;
    }
    if let Some(max_concurrency) = req.max_concurrency {
        separated.push("max_concurrency = ");
        separated.push_bind_unseparated(max_concurrency);
        has_update = true;
    }

    if !has_update {
        return Err(ApiError::bad_request("没有需要更新的字段"));
    }

    separated.push("updated_at = CURRENT_TIMESTAMP");

    builder.push(" WHERE id = ");
    builder.push_bind(&id);

    builder
        .build()
        .execute(&state.pool)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    let channel = get_channel_by_id(&state.pool, &id, state.timezone_offset).await?;
    Ok(Json(ApiResponse::success(channel)))
}

/// 删除渠道
pub async fn delete(
    State(state): State<ChannelState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, (StatusCode, Json<ApiError>)> {
    let result = sqlx::query("DELETE FROM channels WHERE id = ?")
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("渠道不存在"));
    }

    state.cache.invalidate_channel(&id).await;
    Ok(Json(crate::api::response::success_empty()))
}

/// 根据 ID 获取渠道
pub(super) async fn get_channel_by_id(
    pool: &SqlitePool,
    id: &str,
    timezone_offset: i32,
) -> Result<Channel, (StatusCode, Json<ApiError>)> {
    let tz = tz_modifier(timezone_offset);
    let result = sqlx::query_as::<_, ChannelRow>(AssertSqlSafe(
        format!("SELECT id, name, api_keys, endpoints, models, rate_limit_rpm, rate_limit_tpm, failure_threshold, blacklist_minutes, concurrency, timeout_secs, max_concurrency, custom_headers, extras, enabled, datetime(created_at, '{}') as created_at, datetime(updated_at, '{}') as updated_at FROM channels WHERE id = ?", tz, tz)
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::internal_error(e.to_string()))?;

    let row = result.ok_or_else(|| ApiError::not_found("渠道不存在"))?;
    row_to_channel(row).map_err(ApiError::internal_error)
}

pub fn parse_api_keys(json_str: &str) -> Vec<UpstreamApiKey> {
    serde_json::from_str(json_str).unwrap_or_default()
}

fn decode_json_field<T>(field_name: &str, value: &str) -> Result<T, String>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_str(value).map_err(|e| format!("解析 {} 失败: {}", field_name, e))
}

pub(crate) fn row_to_channel(row: ChannelRow) -> Result<Channel, String> {
    let extras: Option<serde_json::Map<String, serde_json::Value>> =
        serde_json::from_str(&row.extras).ok();
    Ok(Channel {
        id: row.id,
        name: row.name,
        api_keys: decode_json_field("channels.api_keys", &row.api_keys)?,
        endpoints: decode_json_field("channels.endpoints", &row.endpoints)?,
        models: decode_json_field("channels.models", &row.models)?,
        rate_limit_rpm: row.rate_limit_rpm,
        rate_limit_tpm: row.rate_limit_tpm,
        failure_threshold: row.failure_threshold,
        blacklist_minutes: row.blacklist_minutes,
        concurrency: row.concurrency,
        timeout_secs: row.timeout_secs,
        max_concurrency: row.max_concurrency,
        custom_headers: decode_json_field("channels.custom_headers", &row.custom_headers)?,
        extras,
        enabled: row.enabled,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

fn row_to_channel_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Channel, String> {
    use sqlx::Row;
    let extras_str: String = row.get("extras");
    let extras: Option<serde_json::Map<String, serde_json::Value>> =
        serde_json::from_str(&extras_str).ok();
    Ok(Channel {
        id: row.get("id"),
        name: row.get("name"),
        api_keys: decode_json_field("channels.api_keys", &row.get::<String, _>("api_keys"))?,
        endpoints: decode_json_field("channels.endpoints", &row.get::<String, _>("endpoints"))?,
        models: decode_json_field("channels.models", &row.get::<String, _>("models"))?,
        rate_limit_rpm: row.get("rate_limit_rpm"),
        rate_limit_tpm: row.get("rate_limit_tpm"),
        failure_threshold: row.get("failure_threshold"),
        blacklist_minutes: row.get("blacklist_minutes"),
        concurrency: row.get("concurrency"),
        timeout_secs: row.get("timeout_secs"),
        max_concurrency: row.get("max_concurrency"),
        custom_headers: decode_json_field(
            "channels.custom_headers",
            &row.get::<String, _>("custom_headers"),
        )?,
        extras,
        enabled: row.get("enabled"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}
