use axum::{Json, extract::State, http::HeaderMap, response::IntoResponse};
use serde_json::Value;

use crate::api::handlers::admin::channels::EndpointType;
use crate::api::middleware::ApiKeyAuth;
use crate::error::proxy::ErrorFormat;
use crate::relay::pipeline::handle_proxy_request;
use crate::relay::state::ProxyState;

/// OpenAI Images 代理
pub async fn proxy(
    State(state): State<ProxyState>,
    auth: ApiKeyAuth,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    handle_proxy_request(
        &state,
        auth,
        headers,
        body,
        &EndpointType::OpenAiImages,
        &ErrorFormat::OpenAi,
    )
    .await
}
