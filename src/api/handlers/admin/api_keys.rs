use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};

use crate::api::handlers::admin::channels::PaginatedResponse;
use crate::app_state::AppState;
use crate::error::app::{ApiError, ApiResponse};

pub use crate::domain::api_key::*;

/// 解析逗号分隔的模型列表。
///
/// 工具函数（非 SQL），relay/proxy 依赖（`relay/pipeline.rs`、`proxy/models.rs`），保留在此层不下沉。
pub fn parse_supported_models(models_str: &str) -> Vec<String> {
    models_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// 获取 API Key 列表（支持搜索、筛选、排序、分页）
pub async fn list(
    State(state): State<AppState>,
    Query(query): Query<ListApiKeysQuery>,
) -> Result<Json<ApiResponse<PaginatedResponse<ApiKey>>>, (StatusCode, Json<ApiError>)> {
    let (items, total) = state
        .repositories
        .api_key
        .list(query)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;
    Ok(Json(ApiResponse::success(PaginatedResponse { items, total })))
}

/// 创建 API Key
pub async fn create(
    State(state): State<AppState>,
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<(StatusCode, Json<ApiResponse<ApiKey>>), (StatusCode, Json<ApiError>)> {
    if req.name.is_empty() {
        return Err(ApiError::bad_request("名称不能为空"));
    }

    let key = state
        .repositories
        .api_key
        .create(req)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;
    Ok((StatusCode::CREATED, Json(ApiResponse::success(key))))
}

/// 获取单个 API Key
pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<ApiKey>>, (StatusCode, Json<ApiError>)> {
    let key = state
        .repositories
        .api_key
        .get(&id)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("API Key 不存在"))?;
    Ok(Json(ApiResponse::success(key)))
}

/// 更新 API Key
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateApiKeyRequest>,
) -> Result<Json<ApiResponse<ApiKey>>, (StatusCode, Json<ApiError>)> {
    let no_fields = req.name.is_none()
        && req.enabled.is_none()
        && req.supported_models.is_none()
        && req.rate_limit_rpm.is_none()
        && req.rate_limit_tpm.is_none()
        && req.allowed_routes.is_none();
    if no_fields {
        return Err(ApiError::bad_request("没有需要更新的字段"));
    }

    let key = state
        .repositories
        .api_key
        .update(&id, req)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("API Key 不存在"))?;

    // 清除缓存，确保下次请求获取最新状态
    state.api_key_cache.invalidate(&key.api_key).await;
    Ok(Json(ApiResponse::success(key)))
}

/// 删除 API Key
pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, (StatusCode, Json<ApiError>)> {
    let api_key_value = state
        .repositories
        .api_key
        .delete(&id)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("API Key 不存在"))?;

    // 清除缓存
    state.api_key_cache.invalidate(&api_key_value).await;
    Ok(Json(crate::error::app::success_empty()))
}
