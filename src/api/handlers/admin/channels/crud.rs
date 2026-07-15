use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};

use super::types::{
    CreateChannelRequest, ListChannelsQuery, PaginatedResponse, UpdateChannelRequest,
};
use crate::domain::channel::Channel;
use crate::repository::channel_repository::row_to_channel;
use crate::app_state::AppState;
use crate::error::app::{ApiError, ApiResponse};

/// 获取渠道列表（支持搜索、筛选、排序、分页）
pub async fn list(
    State(state): State<AppState>,
    Query(query): Query<ListChannelsQuery>,
) -> Result<Json<ApiResponse<PaginatedResponse<Channel>>>, (StatusCode, Json<ApiError>)> {
    let (rows, total) = state
        .repositories
        .channel
        .list(query)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;
    let items: Vec<Channel> = rows
        .into_iter()
        .map(row_to_channel)
        .collect::<Result<Vec<_>, _>>()
        .map_err(ApiError::internal_error)?;
    Ok(Json(ApiResponse::success(PaginatedResponse { items, total })))
}

/// 创建渠道
pub async fn create(
    State(state): State<AppState>,
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

    let row = state
        .repositories
        .channel
        .create(req)
        .await
        .map_err(|e| {
            if e.to_string().contains("UNIQUE constraint failed") {
                ApiError::conflict("渠道名称已存在")
            } else {
                ApiError::internal_error(e.to_string())
            }
        })?;
    state.cache.invalidate_all_channels().await;
    let channel = row_to_channel(row).map_err(ApiError::internal_error)?;
    Ok((StatusCode::CREATED, Json(ApiResponse::success(channel))))
}

/// 获取单个渠道
pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<Channel>>, (StatusCode, Json<ApiError>)> {
    let row = state
        .repositories
        .channel
        .get(&id)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("渠道不存在"))?;
    let channel = row_to_channel(row).map_err(ApiError::internal_error)?;
    Ok(Json(ApiResponse::success(channel)))
}

/// 更新渠道
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateChannelRequest>,
) -> Result<Json<ApiResponse<Channel>>, (StatusCode, Json<ApiError>)> {
    // api_keys 合法性校验
    if let Some(api_keys) = &req.api_keys {
        for k in api_keys {
            crate::relay::pipeline::validate_header_value(&k.key)
                .map_err(|e| ApiError::bad_request(format!("API Key 非法: {e}")))?;
        }
    }

    let has_update = req.name.is_some()
        || req.api_keys.is_some()
        || req.endpoints.is_some()
        || req.models.is_some()
        || req.enabled.is_some()
        || req.rate_limit_rpm.is_some()
        || req.rate_limit_tpm.is_some()
        || req.failure_threshold.is_some()
        || req.blacklist_minutes.is_some()
        || req.concurrency.is_some()
        || req.timeout_secs.is_some()
        || req.max_concurrency.is_some();
    if !has_update {
        return Err(ApiError::bad_request("没有需要更新的字段"));
    }

    let row = state
        .repositories
        .channel
        .update(&id, req)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("渠道不存在"))?;
    let channel = row_to_channel(row).map_err(ApiError::internal_error)?;
    Ok(Json(ApiResponse::success(channel)))
}

/// 删除渠道
pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, (StatusCode, Json<ApiError>)> {
    let route_ids = state
        .repositories
        .channel
        .delete(&id)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("渠道不存在"))?;

    // 清除渠道缓存 + 受影响分组缓存
    state.cache.invalidate_channel(&id).await;
    if !route_ids.is_empty() {
        state.cache.invalidate_all_routes().await;
    }
    Ok(Json(crate::error::app::success_empty()))
}
