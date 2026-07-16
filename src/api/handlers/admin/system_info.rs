use axum::{Json, extract::State};
use axum::http::StatusCode;
use serde::Serialize;

use crate::app_state::AppState;
use crate::error::app::{ApiError, ApiResponse};

#[derive(Debug, Serialize)]
pub struct SystemInfo {
    pub version: &'static str,
    pub uptime_secs: u64,
    pub channel_count: i64,
    pub route_count: i64,
    pub api_key_count: i64,
}

pub async fn get(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<SystemInfo>>, (StatusCode, Json<ApiError>)> {
    let (channel_count, route_count, api_key_count) = state
        .repositories
        .system_info
        .count_summary()
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    let uptime_secs = state.start_time.elapsed().as_secs();

    Ok(Json(ApiResponse::success(SystemInfo {
        version: env!("GALAXY_BUILD_VERSION"),
        uptime_secs,
        channel_count,
        route_count,
        api_key_count,
    })))
}

/// 手动 VACUUM 回收磁盘空间（retention 删日志后释放）。需无活跃事务，建议低峰调用。
pub async fn vacuum(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<()>>, (StatusCode, Json<ApiError>)> {
    state
        .repositories
        .system_info
        .vacuum()
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;
    Ok(Json(crate::error::app::success_empty()))
}
