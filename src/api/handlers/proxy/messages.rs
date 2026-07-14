use axum::{Json, extract::State, http::HeaderMap, response::IntoResponse};
use serde_json::Value;

use crate::api::handlers::admin::channels::EndpointType;
use crate::api::middleware::ApiKeyAuth;
use crate::error::proxy::ErrorFormat;
use crate::relay::pipeline::handle_proxy_request;
use crate::app_state::AppState;

/// Anthropic Messages 代理
pub async fn proxy(
    State(state): State<AppState>,
    auth: ApiKeyAuth,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    handle_proxy_request(
        &state,
        auth,
        headers,
        body,
        &EndpointType::Anthropic,
        &ErrorFormat::Anthropic,
    )
    .await
}
