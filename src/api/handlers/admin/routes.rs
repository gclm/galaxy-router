use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};

use crate::api::handlers::admin::channels::PaginatedResponse;
use crate::app_state::AppState;
use crate::error::app::{ApiError, ApiResponse};
use crate::service::route::{RouteError, RouteService};

pub use crate::domain::route::*;

fn map_route_err(e: RouteError) -> (StatusCode, Json<ApiError>) {
    match e {
        RouteError::BadRequest(m) => ApiError::bad_request(m),
        RouteError::Conflict(m) => ApiError::conflict(m),
        RouteError::NotFound(m) => ApiError::not_found(m),
        RouteError::Internal(m) => ApiError::internal_error(m),
    }
}

fn route_service(state: &AppState) -> RouteService {
    RouteService::new(state.repositories.route.clone(), state.cache.clone())
}

/// 获取分组列表（支持搜索、筛选、排序、分页）
pub async fn list(
    State(state): State<AppState>,
    Query(query): Query<ListRoutesQuery>,
) -> Result<Json<ApiResponse<PaginatedResponse<Route>>>, (StatusCode, Json<ApiError>)> {
    let (items, total) = route_service(&state)
        .list(query)
        .await
        .map_err(map_route_err)?;
    Ok(Json(ApiResponse::success(PaginatedResponse { items, total })))
}

/// 创建分组
pub async fn create(
    State(state): State<AppState>,
    Json(req): Json<CreateRouteRequest>,
) -> Result<(StatusCode, Json<ApiResponse<Route>>), (StatusCode, Json<ApiError>)> {
    let route = route_service(&state)
        .create(req)
        .await
        .map_err(map_route_err)?;
    Ok((StatusCode::CREATED, Json(ApiResponse::success(route))))
}

/// 获取单个分组
pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<Route>>, (StatusCode, Json<ApiError>)> {
    let route = route_service(&state)
        .get(&id)
        .await
        .map_err(map_route_err)?;
    Ok(Json(ApiResponse::success(route)))
}

/// 更新分组
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateRouteRequest>,
) -> Result<Json<ApiResponse<Route>>, (StatusCode, Json<ApiError>)> {
    let route = route_service(&state)
        .update(&id, req)
        .await
        .map_err(map_route_err)?;
    Ok(Json(ApiResponse::success(route)))
}

/// 删除分组
pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, (StatusCode, Json<ApiError>)> {
    route_service(&state)
        .delete(&id)
        .await
        .map_err(map_route_err)?;
    Ok(Json(crate::error::app::success_empty()))
}

/// 添加分组项
pub async fn add_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<AddRouteItemRequest>,
) -> Result<(StatusCode, Json<ApiResponse<RouteItem>>), (StatusCode, Json<ApiError>)> {
    let item = route_service(&state)
        .add_item(&id, req)
        .await
        .map_err(map_route_err)?;
    Ok((StatusCode::CREATED, Json(ApiResponse::success(item))))
}

/// 删除分组项
pub async fn delete_item(
    State(state): State<AppState>,
    Path((route_id, item_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<()>>, (StatusCode, Json<ApiError>)> {
    route_service(&state)
        .delete_item(&route_id, &item_id)
        .await
        .map_err(map_route_err)?;
    Ok(Json(crate::error::app::success_empty()))
}
