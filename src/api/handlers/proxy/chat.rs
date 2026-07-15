use axum::{Json, extract::State, http::HeaderMap, response::IntoResponse};
use serde_json::Value;

use crate::domain::channel::EndpointType;
use crate::api::middleware::ApiKeyAuth;
use crate::error::proxy::ErrorFormat;
use crate::llm::relay::pipeline::handle_proxy_request;
use crate::app_state::AppState;

/// OpenAI Chat Completions 代理
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
        &EndpointType::OpenAiChat,
        &ErrorFormat::OpenAi,
    )
    .await
}
