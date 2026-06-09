use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};
use futures::Stream;
use std::convert::Infallible;
use std::pin::Pin;

use crate::api::handlers::admin::channels::EndpointType;
use crate::proxy::execute::AttemptStats;
use crate::proxy::selection::SelectionResult;
use crate::proxy::{ProxyError, ProxyState};

pub(crate) type RelayBodyStream = Pin<Box<dyn Stream<Item = Result<Bytes, Infallible>> + Send>>;

/// Execute one streaming upstream attempt.
///
/// P12 bridge: the implementation still delegates to `proxy::execute` until
/// SSE/observability/state ownership is moved in P13-P15. Callers should use
/// this relay entrypoint so the execution boundary is already relay-owned.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_once(
    state: &ProxyState,
    request_id: String,
    api_key_id: Option<&str>,
    upstream_api_key: &str,
    key_hint: String,
    group_id: Option<String>,
    headers: &HeaderMap,
    body: &serde_json::Value,
    client_endpoint: &EndpointType,
    selection: &SelectionResult,
    attempts: &mut Vec<AttemptStats>,
    queue_permit: Option<tokio::sync::OwnedSemaphorePermit>,
) -> Result<(StatusCode, RelayBodyStream, String, Option<i32>), ProxyError> {
    crate::proxy::execute::execute_proxy_stream(
        state,
        request_id,
        api_key_id,
        upstream_api_key,
        key_hint,
        group_id,
        headers,
        body,
        client_endpoint,
        selection,
        attempts,
        queue_permit,
    )
    .await
}
