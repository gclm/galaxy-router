//! 模型列表发现 handler（薄：调 service/discovery）。多端点回退 + 协议分页在 service（D4 归位）。

use axum::{Json, extract::State, http::StatusCode};
use serde::Deserialize;

use crate::app_state::AppState;
use crate::domain::channel::EndpointConfig;
use crate::error::app::{ApiError, ApiResponse};
use crate::service::discovery::{FetchError, ModelDiscovery};

/// 获取模型列表请求
#[derive(Debug, Deserialize)]
pub struct FetchModelsRequest {
    pub endpoints: Vec<EndpointConfig>,
    pub api_key: String,
}

/// 获取模型列表
pub async fn fetch_models(
    State(state): State<AppState>,
    Json(req): Json<FetchModelsRequest>,
) -> Result<Json<ApiResponse<Vec<String>>>, (StatusCode, Json<ApiError>)> {
    if req.endpoints.is_empty() {
        return Err(ApiError::bad_request("至少需要一个端点"));
    }

    let discovery = ModelDiscovery::new(state.models_http_client.clone());
    match discovery.fetch_models(&req.endpoints, &req.api_key).await {
        Ok(models) => Ok(Json(ApiResponse::success(models))),
        Err(FetchError::AuthFailed) => Err(ApiError::bad_request("API Key 无效")),
        Err(FetchError::Http(status)) => Err(ApiError::internal_error(format!(
            "所有端点均失败，最后错误: HTTP {}",
            status
        ))),
        Err(FetchError::Other(e)) => {
            Err(ApiError::internal_error(format!("获取模型列表失败: {}", e)))
        }
    }
}
