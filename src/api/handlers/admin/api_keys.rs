use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, Row, SqlitePool};

use crate::api::handlers::admin::channels::PaginatedResponse;
use crate::api::middleware::ApiKeyCache;
use crate::api::response::generate_id;
use crate::error::app::{ApiError, ApiResponse};
use crate::util::timeutil::{now_local_str, tz_modifier};

/// API Key
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ApiKey {
    pub id: String,
    pub name: String,
    pub api_key: String,
    pub enabled: bool,
    pub supported_models: Option<String>,
    pub rate_limit_rpm: i32,
    pub rate_limit_tpm: i32,
    pub allowed_routes: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// 创建 API Key 请求
#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: String,
    pub supported_models: Option<String>,
    pub rate_limit_rpm: Option<i32>,
    pub rate_limit_tpm: Option<i32>,
    pub allowed_routes: Option<String>,
}

/// 更新 API Key 请求
#[derive(Debug, Deserialize)]
pub struct UpdateApiKeyRequest {
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub supported_models: Option<String>,
    pub rate_limit_rpm: Option<i32>,
    pub rate_limit_tpm: Option<i32>,
    pub allowed_routes: Option<String>,
}

/// API Key 状态
#[derive(Clone)]
pub struct ApiKeyState {
    pub pool: SqlitePool,
    pub api_key_cache: ApiKeyCache,
    pub timezone_offset: i32,
}

/// 列表查询参数
#[derive(Debug, Deserialize)]
pub struct ListApiKeysQuery {
    pub search: Option<String>,
    pub status: Option<String>,
    pub sort_by: Option<String>,
    pub sort_order: Option<String>,
    pub page: Option<i32>,
    pub page_size: Option<i32>,
}

/// 空字符串转 None
fn empty_to_none(s: String) -> Option<String> {
    if s.is_empty() { None } else { Some(s) }
}

/// 解析逗号分隔的模型列表
pub fn parse_supported_models(models_str: &str) -> Vec<String> {
    models_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// 获取 API Key 列表（支持搜索、筛选、排序、分页）
pub async fn list(
    State(state): State<ApiKeyState>,
    Query(query): Query<ListApiKeysQuery>,
) -> Result<Json<ApiResponse<PaginatedResponse<ApiKey>>>, (StatusCode, Json<ApiError>)> {
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

    // Count
    let mut count_builder = sqlx::QueryBuilder::new("SELECT COUNT(*) FROM api_keys");
    let _has_where = push_where(&mut count_builder, &query);
    let count_row = count_builder
        .build()
        .fetch_one(&state.pool)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;
    let total: i64 = sqlx::Row::get(&count_row, 0);

    // Data
    let tz = tz_modifier(state.timezone_offset);
    let mut data_builder = sqlx::QueryBuilder::new(format!(
        "SELECT id, name, api_key, enabled, COALESCE(supported_models, '') as supported_models, COALESCE(rate_limit_rpm, 0) as rate_limit_rpm, COALESCE(rate_limit_tpm, 0) as rate_limit_tpm, COALESCE(allowed_routes, '') as allowed_routes, datetime(created_at, '{}') as created_at, datetime(updated_at, '{}') as updated_at FROM api_keys",
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

    let items: Vec<ApiKey> = rows
        .iter()
        .map(|row| ApiKey {
            id: row.get("id"),
            name: row.get("name"),
            api_key: row.get("api_key"),
            enabled: row.get("enabled"),
            supported_models: empty_to_none(row.get::<String, _>("supported_models")),
            rate_limit_rpm: row.get("rate_limit_rpm"),
            rate_limit_tpm: row.get("rate_limit_tpm"),
            allowed_routes: empty_to_none(row.get::<String, _>("allowed_routes")),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        })
        .collect();

    Ok(Json(ApiResponse::success(PaginatedResponse { items, total })))
}

fn push_where(
    builder: &mut sqlx::QueryBuilder<sqlx::Sqlite>,
    query: &ListApiKeysQuery,
) -> bool {
    let mut has_where = false;

    if let Some(ref search) = query.search
        && !search.is_empty()
    {
        builder.push(" WHERE name LIKE ");
        builder.push_bind(format!("%{search}%"));
        has_where = true;
    }

    if let Some(ref status) = query.status {
        if has_where {
            builder.push(" AND enabled = ");
        } else {
            builder.push(" WHERE enabled = ");
        }
        builder.push_bind(status == "enabled");
        has_where = true;
    }

    has_where
}

/// 创建 API Key
pub async fn create(
    State(state): State<ApiKeyState>,
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<(StatusCode, Json<ApiResponse<ApiKey>>), (StatusCode, Json<ApiError>)> {
    // 验证输入
    if req.name.is_empty() {
        return Err(ApiError::bad_request("名称不能为空"));
    }

    let id = generate_id();
    let api_key = format!("sk-gr-{}", generate_id());
    let supported_models = req.supported_models.unwrap_or_default();
    let rate_limit_rpm = req.rate_limit_rpm.unwrap_or(0);
    let rate_limit_tpm = req.rate_limit_tpm.unwrap_or(0);
    let allowed_routes = req.allowed_routes.unwrap_or_default();

    // 插入 API Key
    sqlx::query(
        r#"
        INSERT INTO api_keys (id, name, api_key, enabled, supported_models, rate_limit_rpm, rate_limit_tpm, allowed_routes)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(&req.name)
    .bind(&api_key)
    .bind(true)
    .bind(&supported_models)
    .bind(rate_limit_rpm)
    .bind(rate_limit_tpm)
    .bind(&allowed_routes)
    .execute(&state.pool)
    .await
    .map_err(|e| ApiError::internal_error(e.to_string()))?;

    let key = ApiKey {
        id,
        name: req.name,
        api_key,
        enabled: true,
        supported_models: if supported_models.is_empty() {
            None
        } else {
            Some(supported_models)
        },
        rate_limit_rpm,
        rate_limit_tpm,
        allowed_routes: if allowed_routes.is_empty() {
            None
        } else {
            Some(allowed_routes)
        },
        created_at: now_local_str(state.timezone_offset),
        updated_at: now_local_str(state.timezone_offset),
    };

    Ok((StatusCode::CREATED, Json(ApiResponse::success(key))))
}

/// 获取单个 API Key
pub async fn get(
    State(state): State<ApiKeyState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<ApiKey>>, (StatusCode, Json<ApiError>)> {
    let tz = tz_modifier(state.timezone_offset);
    let result = sqlx::query_as::<_, (String, String, String, bool, String, i32, i32, String, String, String)>(
        AssertSqlSafe(format!("SELECT id, name, api_key, enabled, supported_models, COALESCE(rate_limit_rpm, 0), COALESCE(rate_limit_tpm, 0), COALESCE(allowed_routes, ''), datetime(created_at, '{tz}') as created_at, datetime(updated_at, '{tz}') as updated_at FROM api_keys WHERE id = ?").as_str()),
    )
    .bind(&id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| ApiError::internal_error(e.to_string()))?;

    let (
        id,
        name,
        api_key,
        enabled,
        supported_models,
        rate_limit_rpm,
        rate_limit_tpm,
        allowed_routes,
        created_at,
        updated_at,
    ) = result.ok_or_else(|| ApiError::not_found("API Key 不存在"))?;

    Ok(Json(ApiResponse::success(ApiKey {
        id,
        name,
        api_key,
        enabled,
        supported_models: empty_to_none(supported_models),
        rate_limit_rpm,
        rate_limit_tpm,
        allowed_routes: empty_to_none(allowed_routes),
        created_at,
        updated_at,
    })))
}

/// 更新 API Key
pub async fn update(
    State(state): State<ApiKeyState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateApiKeyRequest>,
) -> Result<Json<ApiResponse<ApiKey>>, (StatusCode, Json<ApiError>)> {
    // 检查 API Key 是否存在，并获取当前 api_key 值用于清除缓存
    let existing = sqlx::query_scalar::<_, String>("SELECT api_key FROM api_keys WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.pool)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    let api_key_value = existing.ok_or_else(|| ApiError::not_found("API Key 不存在"))?;

    // 构建更新语句
    let mut builder = sqlx::QueryBuilder::new("UPDATE api_keys SET ");
    let mut separated = builder.separated(", ");
    let mut has_update = false;

    if let Some(ref name) = req.name {
        separated.push("name = ");
        separated.push_bind_unseparated(name);
        has_update = true;
    }
    if let Some(enabled) = req.enabled {
        separated.push("enabled = ");
        separated.push_bind_unseparated(enabled);
        has_update = true;
    }
    if let Some(ref supported_models) = req.supported_models {
        separated.push("supported_models = ");
        separated.push_bind_unseparated(supported_models);
        has_update = true;
    }
    if let Some(rpm) = req.rate_limit_rpm {
        separated.push("rate_limit_rpm = ");
        separated.push_bind_unseparated(rpm);
        has_update = true;
    }
    if let Some(tpm) = req.rate_limit_tpm {
        separated.push("rate_limit_tpm = ");
        separated.push_bind_unseparated(tpm);
        has_update = true;
    }
    if let Some(ref allowed_routes) = req.allowed_routes {
        separated.push("allowed_routes = ");
        separated.push_bind_unseparated(allowed_routes);
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

    // 清除缓存，确保下次请求获取最新状态
    state.api_key_cache.invalidate(&api_key_value).await;

    // 返回更新后的 API Key
    get(State(state), Path(id)).await
}

/// 删除 API Key
pub async fn delete(
    State(state): State<ApiKeyState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, (StatusCode, Json<ApiError>)> {
    // 先获取 api_key 值用于清除缓存
    let api_key_value =
        sqlx::query_scalar::<_, String>("SELECT api_key FROM api_keys WHERE id = ?")
            .bind(&id)
            .fetch_optional(&state.pool)
            .await
            .map_err(|e| ApiError::internal_error(e.to_string()))?;

    let api_key_value = api_key_value.ok_or_else(|| ApiError::not_found("API Key 不存在"))?;

    let result = sqlx::query("DELETE FROM api_keys WHERE id = ?")
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("API Key 不存在"));
    }

    // 清除缓存
    state.api_key_cache.invalidate(&api_key_value).await;

    Ok(Json(crate::error::app::success_empty()))
}
