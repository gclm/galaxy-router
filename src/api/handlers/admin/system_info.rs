use axum::{Json, extract::State};
use serde::Serialize;
use sqlx::SqlitePool;
use std::sync::Arc;
use std::time::Instant;

use crate::error::app::{ApiError, ApiResponse};
use axum::http::StatusCode;

#[derive(Clone)]
pub struct SystemInfoState {
    pub pool: SqlitePool,
    pub start_time: Arc<Instant>,
}

#[derive(Debug, Serialize)]
pub struct SystemInfo {
    pub version: &'static str,
    pub uptime_secs: u64,
    pub channel_count: i64,
    pub route_count: i64,
    pub api_key_count: i64,
}

pub async fn get(
    State(state): State<SystemInfoState>,
) -> Result<Json<ApiResponse<SystemInfo>>, (StatusCode, Json<ApiError>)> {
    let channel_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM channels")
        .fetch_one(&state.pool)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    let route_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM routes")
        .fetch_one(&state.pool)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    let api_key_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM api_keys")
        .fetch_one(&state.pool)
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
