use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, SqlitePool};

use crate::api::middleware::ApiKeyCache;
use crate::api::{ApiError, ApiResponse, response::generate_id};
use crate::metrics::query::{now_local_str, tz_modifier};

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
    pub allowed_groups: Option<String>,
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
    pub allowed_groups: Option<String>,
}

/// 更新 API Key 请求
#[derive(Debug, Deserialize)]
pub struct UpdateApiKeyRequest {
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub supported_models: Option<String>,
    pub rate_limit_rpm: Option<i32>,
    pub rate_limit_tpm: Option<i32>,
    pub allowed_groups: Option<String>,
}

/// API Key 状态
#[derive(Clone)]
pub struct ApiKeyState {
    pub pool: SqlitePool,
    pub api_key_cache: ApiKeyCache,
    pub timezone_offset: i32,
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

/// 获取 API Key 列表
pub async fn list(
    State(state): State<ApiKeyState>,
) -> Result<Json<ApiResponse<Vec<ApiKey>>>, (StatusCode, Json<ApiError>)> {
    let tz = tz_modifier(state.timezone_offset);
    let keys = sqlx::query_as::<_, (String, String, String, bool, String, i32, i32, String, String, String)>(
        AssertSqlSafe(format!("SELECT id, name, api_key, enabled, supported_models, COALESCE(rate_limit_rpm, 0), COALESCE(rate_limit_tpm, 0), COALESCE(allowed_groups, ''), datetime(created_at, '{tz}') as created_at, datetime(updated_at, '{tz}') as updated_at FROM api_keys ORDER BY created_at DESC").as_str()),
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| ApiError::internal_error(e.to_string()))?;

    let result: Vec<ApiKey> = keys
        .into_iter()
        .map(
            |(
                id,
                name,
                api_key,
                enabled,
                supported_models,
                rate_limit_rpm,
                rate_limit_tpm,
                allowed_groups,
                created_at,
                updated_at,
            )| ApiKey {
                id,
                name,
                api_key,
                enabled,
                supported_models: empty_to_none(supported_models),
                rate_limit_rpm,
                rate_limit_tpm,
                allowed_groups: empty_to_none(allowed_groups),
                created_at,
                updated_at,
            },
        )
        .collect();

    Ok(Json(ApiResponse::success(result)))
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
    let api_key = format!("gp-{}", generate_id());
    let supported_models = req.supported_models.unwrap_or_default();
    let rate_limit_rpm = req.rate_limit_rpm.unwrap_or(0);
    let rate_limit_tpm = req.rate_limit_tpm.unwrap_or(0);
    let allowed_groups = req.allowed_groups.unwrap_or_default();

    // 插入 API Key
    sqlx::query(
        r#"
        INSERT INTO api_keys (id, name, api_key, enabled, supported_models, rate_limit_rpm, rate_limit_tpm, allowed_groups)
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
    .bind(&allowed_groups)
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
        allowed_groups: if allowed_groups.is_empty() {
            None
        } else {
            Some(allowed_groups)
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
        AssertSqlSafe(format!("SELECT id, name, api_key, enabled, supported_models, COALESCE(rate_limit_rpm, 0), COALESCE(rate_limit_tpm, 0), COALESCE(allowed_groups, ''), datetime(created_at, '{tz}') as created_at, datetime(updated_at, '{tz}') as updated_at FROM api_keys WHERE id = ?").as_str()),
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
        allowed_groups,
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
        allowed_groups: empty_to_none(allowed_groups),
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
    if let Some(ref allowed_groups) = req.allowed_groups {
        separated.push("allowed_groups = ");
        separated.push_bind_unseparated(allowed_groups);
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

    Ok(Json(crate::api::response::success_empty()))
}
