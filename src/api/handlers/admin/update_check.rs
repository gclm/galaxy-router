//! 版本更新检查 handler（薄：调 service.update_check）。
//!
//! service 逻辑（UpdateCheckContext / check / fetch_and_compare）在 service/update_check（D1 归位）。

use axum::{Json, extract::State, http::StatusCode};

use crate::app_state::AppState;
use crate::error::app::{ApiError, ApiResponse};
use crate::service::update_check::UpdateCheckResponse;

/// 检查更新
pub async fn get(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<UpdateCheckResponse>>, (StatusCode, Json<ApiError>)> {
    let response = state
        .update_check
        .check()
        .await
        .map_err(ApiError::internal_error)?;
    Ok(Json(ApiResponse::success(response)))
}
