use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use crate::app_state::AppState;
use crate::domain::pricing::UpdateModelRequest;
use crate::error::app::{ApiError, ApiResponse};

/// 获取所有模型信息
pub async fn list(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<Vec<crate::domain::pricing::ModelInfo>>>, (StatusCode, Json<ApiError>)>
{
    let models = state.model_registry.get_all_models().await;
    Ok(Json(ApiResponse::success(models)))
}

/// 获取指定模型信息
pub async fn get(
    State(state): State<AppState>,
    Path(model): Path<String>,
) -> Result<Json<ApiResponse<crate::domain::pricing::ModelInfo>>, (StatusCode, Json<ApiError>)> {
    match state.model_registry.get_model_info(&model).await {
        Some(info) => Ok(Json(ApiResponse::success(info))),
        None => Err(ApiError::not_found(format!("模型 {} 不存在", model))),
    }
}

/// 更新模型信息
pub async fn update(
    State(state): State<AppState>,
    Json(req): Json<UpdateModelRequest>,
) -> Result<Json<ApiResponse<crate::domain::pricing::ModelInfo>>, (StatusCode, Json<ApiError>)> {
    let info = state
        .model_registry
        .update_model_info(req)
        .await
        .map_err(ApiError::internal_error)?;
    Ok(Json(ApiResponse::success(info)))
}
