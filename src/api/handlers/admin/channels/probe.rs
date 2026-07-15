use axum::{Json, extract::State, http::StatusCode};

use crate::app_state::AppState;
use crate::error::app::{ApiError, ApiResponse};
use crate::service::channel::probe::{ChannelProbe, TestEndpointRequest, TestEndpointResponse};

/// 端点测试：对指定端点发流式探测请求，返回连通性 + 思维链诊断（per-endpoint，不合并）。
///
/// Err 仅用于协议不支持（→ 400）；网络/上游失败进 Response.error（success=false）。
pub async fn test_endpoint(
    State(state): State<AppState>,
    Json(req): Json<TestEndpointRequest>,
) -> Result<Json<ApiResponse<TestEndpointResponse>>, (StatusCode, Json<ApiError>)> {
    let resp = ChannelProbe::new(state.channel_http_client.clone())
        .test_endpoint(req)
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(ApiResponse::success(resp)))
}
