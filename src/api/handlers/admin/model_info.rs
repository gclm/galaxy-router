use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;

use crate::api::{ApiError, ApiResponse};
use crate::metrics::model::ModelRegistry;

/// 定价 API 状态
#[derive(Clone)]
pub struct ModelInfoState {
    pub model_registry: ModelRegistry,
}

/// 获取所有模型信息
pub async fn list(
    State(state): State<ModelInfoState>,
) -> Result<Json<ApiResponse<Vec<crate::metrics::model::ModelInfo>>>, (StatusCode, Json<ApiError>)>
{
    let models = state.model_registry.get_all_models().await;
    Ok(Json(ApiResponse::success(models)))
}

/// 获取指定模型信息
pub async fn get(
    State(state): State<ModelInfoState>,
    Path(model): Path<String>,
) -> Result<Json<ApiResponse<crate::metrics::model::ModelInfo>>, (StatusCode, Json<ApiError>)> {
    match state.model_registry.get_model_info(&model).await {
        Some(info) => Ok(Json(ApiResponse::success(info))),
        None => Err(ApiError::not_found(format!("模型 {} 不存在", model))),
    }
}

/// 更新模型信息请求
#[derive(Debug, Deserialize)]
pub struct UpdateModelRequest {
    pub model: String,
    pub provider: Option<String>,
    pub mode: Option<String>,
    pub input_price: Option<f64>,
    pub output_price: Option<f64>,
    pub cache_read_price: Option<f64>,
    pub cache_creation_price: Option<f64>,
    pub max_input_tokens: Option<i64>,
    pub max_output_tokens: Option<i64>,
    pub supports_function_calling: Option<bool>,
    pub supports_reasoning: Option<bool>,
    pub supports_vision: Option<bool>,
    pub supports_pdf_input: Option<bool>,
    pub supports_prompt_caching: Option<bool>,
    pub supports_system_messages: Option<bool>,
    pub supports_tool_choice: Option<bool>,
}

/// 更新模型信息
pub async fn update(
    State(state): State<ModelInfoState>,
    Json(req): Json<UpdateModelRequest>,
) -> Result<Json<ApiResponse<crate::metrics::model::ModelInfo>>, (StatusCode, Json<ApiError>)> {
    // 先获取现有信息用于合并
    let existing = state.model_registry.get_model_info(&req.model).await;
    let existing = existing.unwrap_or_else(|| crate::metrics::model::ModelInfo {
        model: req.model.clone(),
        provider: String::new(),
        mode: "chat".to_string(),
        input_price: None,
        output_price: None,
        cache_read_price: None,
        cache_creation_price: None,
        max_input_tokens: None,
        max_output_tokens: None,
        supports_function_calling: None,
        supports_reasoning: None,
        supports_vision: None,
        supports_pdf_input: None,
        supports_prompt_caching: None,
        supports_system_messages: None,
        supports_tool_choice: None,
    });

    let info = crate::metrics::model::ModelInfo {
        model: req.model,
        provider: req.provider.unwrap_or(existing.provider),
        mode: req.mode.unwrap_or(existing.mode),
        input_price: req.input_price.or(existing.input_price),
        output_price: req.output_price.or(existing.output_price),
        cache_read_price: req.cache_read_price.or(existing.cache_read_price),
        cache_creation_price: req.cache_creation_price.or(existing.cache_creation_price),
        max_input_tokens: req.max_input_tokens.or(existing.max_input_tokens),
        max_output_tokens: req.max_output_tokens.or(existing.max_output_tokens),
        supports_function_calling: req
            .supports_function_calling
            .or(existing.supports_function_calling),
        supports_reasoning: req.supports_reasoning.or(existing.supports_reasoning),
        supports_vision: req.supports_vision.or(existing.supports_vision),
        supports_pdf_input: req.supports_pdf_input.or(existing.supports_pdf_input),
        supports_prompt_caching: req
            .supports_prompt_caching
            .or(existing.supports_prompt_caching),
        supports_system_messages: req
            .supports_system_messages
            .or(existing.supports_system_messages),
        supports_tool_choice: req.supports_tool_choice.or(existing.supports_tool_choice),
    };

    state
        .model_registry
        .set_model_info(info.clone())
        .await
        .map_err(ApiError::internal_error)?;

    Ok(Json(ApiResponse::success(info)))
}
