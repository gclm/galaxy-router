use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};

use crate::api::handlers::admin::channels::PaginatedResponse;
use crate::app_state::AppState;
use crate::error::app::{ApiError, ApiResponse};

pub use crate::domain::route::*;

/// 获取分组列表（支持搜索、筛选、排序、分页）
pub async fn list(
    State(state): State<AppState>,
    Query(query): Query<ListRoutesQuery>,
) -> Result<Json<ApiResponse<PaginatedResponse<Route>>>, (StatusCode, Json<ApiError>)> {
    let (items, total) = state
        .repositories
        .route
        .list(query)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;
    Ok(Json(ApiResponse::success(PaginatedResponse { items, total })))
}

/// 创建分组
pub async fn create(
    State(state): State<AppState>,
    Json(req): Json<CreateRouteRequest>,
) -> Result<(StatusCode, Json<ApiResponse<Route>>), (StatusCode, Json<ApiError>)> {
    if req.name.is_empty() {
        return Err(ApiError::bad_request("分组名称不能为空"));
    }
    if req.items.is_empty() {
        return Err(ApiError::bad_request("至少需要一个分组项"));
    }

    let route = state
        .repositories
        .route
        .create(req)
        .await
        .map_err(|e| {
            let s = e.to_string();
            if s.contains("UNIQUE constraint failed") {
                ApiError::conflict("分组名称已存在")
            } else if s.contains("FOREIGN KEY constraint failed") {
                ApiError::bad_request("渠道不存在")
            } else {
                ApiError::internal_error(s)
            }
        })?;

    state.cache.invalidate_all_routes().await;
    Ok((StatusCode::CREATED, Json(ApiResponse::success(route))))
}

/// 获取单个分组
pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<Route>>, (StatusCode, Json<ApiError>)> {
    let route = state
        .repositories
        .route
        .get(&id)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("分组不存在"))?;
    Ok(Json(ApiResponse::success(route)))
}

/// 更新分组
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateRouteRequest>,
) -> Result<Json<ApiResponse<Route>>, (StatusCode, Json<ApiError>)> {
    if let Some(items) = &req.items
        && items.is_empty()
    {
        return Err(ApiError::bad_request("至少需要一个分组项"));
    }

    let route = state
        .repositories
        .route
        .update(&id, req)
        .await
        .map_err(|e| {
            let s = e.to_string();
            if s.contains("FOREIGN KEY constraint failed") {
                ApiError::bad_request("渠道不存在")
            } else if s.contains("UNIQUE constraint failed") {
                ApiError::conflict("分组项重复")
            } else {
                ApiError::internal_error(s)
            }
        })?
        .ok_or_else(|| ApiError::not_found("分组不存在"))?;

    state.cache.invalidate_all_routes().await;
    Ok(Json(ApiResponse::success(route)))
}

/// 删除分组
pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, (StatusCode, Json<ApiError>)> {
    let deleted = state
        .repositories
        .route
        .delete(&id)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    if !deleted {
        return Err(ApiError::not_found("分组不存在"));
    }

    state.cache.invalidate_all_routes().await;
    Ok(Json(crate::error::app::success_empty()))
}

/// 添加分组项
pub async fn add_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<AddRouteItemRequest>,
) -> Result<(StatusCode, Json<ApiResponse<RouteItem>>), (StatusCode, Json<ApiError>)> {
    let item = state
        .repositories
        .route
        .add_item(&id, req)
        .await
        .map_err(|e| {
            let s = e.to_string();
            if s.contains("FOREIGN KEY constraint failed") {
                ApiError::bad_request("渠道不存在")
            } else {
                ApiError::internal_error(s)
            }
        })?
        .ok_or_else(|| ApiError::not_found("分组不存在"))?;

    Ok((StatusCode::CREATED, Json(ApiResponse::success(item))))
}

/// 删除分组项
pub async fn delete_item(
    State(state): State<AppState>,
    Path((route_id, item_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<()>>, (StatusCode, Json<ApiError>)> {
    let deleted = state
        .repositories
        .route
        .delete_item(&route_id, &item_id)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    if !deleted {
        return Err(ApiError::not_found("分组项不存在"));
    }

    state.cache.invalidate_all_routes().await;
    Ok(Json(crate::error::app::success_empty()))
}
