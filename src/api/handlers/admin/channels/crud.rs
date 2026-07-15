use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};

use super::types::PaginatedResponse;
use crate::app_state::AppState;
use crate::domain::channel::{
    Channel, CreateChannelRequest, ListChannelsQuery, UpdateChannelRequest,
};
use crate::error::app::{ApiError, ApiResponse};
use crate::service::channel::{ChannelError, ChannelService};

fn map_channel_err(e: ChannelError) -> (StatusCode, Json<ApiError>) {
    match e {
        ChannelError::BadRequest(m) => ApiError::bad_request(m),
        ChannelError::Conflict(m) => ApiError::conflict(m),
        ChannelError::NotFound(m) => ApiError::not_found(m),
        ChannelError::Internal(m) => ApiError::internal_error(m),
    }
}

fn channel_service(state: &AppState) -> ChannelService {
    ChannelService::new(state.repositories.channel.clone(), state.cache.clone())
}

/// 获取渠道列表（支持搜索、筛选、排序、分页）
pub async fn list(
    State(state): State<AppState>,
    Query(query): Query<ListChannelsQuery>,
) -> Result<Json<ApiResponse<PaginatedResponse<Channel>>>, (StatusCode, Json<ApiError>)> {
    let (items, total) = channel_service(&state)
        .list(query)
        .await
        .map_err(map_channel_err)?;
    Ok(Json(ApiResponse::success(PaginatedResponse { items, total })))
}

/// 创建渠道
pub async fn create(
    State(state): State<AppState>,
    Json(req): Json<CreateChannelRequest>,
) -> Result<(StatusCode, Json<ApiResponse<Channel>>), (StatusCode, Json<ApiError>)> {
    let channel = channel_service(&state)
        .create(req)
        .await
        .map_err(map_channel_err)?;
    Ok((StatusCode::CREATED, Json(ApiResponse::success(channel))))
}

/// 获取单个渠道
pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<Channel>>, (StatusCode, Json<ApiError>)> {
    let channel = channel_service(&state)
        .get(&id)
        .await
        .map_err(map_channel_err)?;
    Ok(Json(ApiResponse::success(channel)))
}

/// 更新渠道
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateChannelRequest>,
) -> Result<Json<ApiResponse<Channel>>, (StatusCode, Json<ApiError>)> {
    let channel = channel_service(&state)
        .update(&id, req)
        .await
        .map_err(map_channel_err)?;
    Ok(Json(ApiResponse::success(channel)))
}

/// 删除渠道
pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, (StatusCode, Json<ApiError>)> {
    channel_service(&state)
        .delete(&id)
        .await
        .map_err(map_channel_err)?;
    Ok(Json(crate::error::app::success_empty()))
}
