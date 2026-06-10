use axum::{Json, extract::State, http::HeaderMap, response::IntoResponse};
use serde_json::Value;

use crate::api::handlers::admin::channels::EndpointType;
use crate::api::middleware::ApiKeyAuth;
use crate::relay::state::{self, ErrorFormat, ProxyState};

/// OpenAI Embeddings 代理
pub async fn proxy(
    State(state): State<ProxyState>,
    auth: ApiKeyAuth,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    state::handle_proxy_request(
        &state,
        auth,
        headers,
        body,
        &EndpointType::OpenAiEmbedding,
        &ErrorFormat::OpenAi,
    )
    .await
}
