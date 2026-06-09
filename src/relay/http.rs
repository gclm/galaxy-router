use crate::api::handlers::admin::channels::EndpointType;
use crate::proxy::execute::AttemptStats;
use crate::proxy::selection::SelectionResult;
use crate::proxy::{ProxyError, ProxyState, ProxySuccess};
use axum::http::HeaderMap;

/// Execute one non-stream upstream attempt.
///
/// P12 bridge: the implementation still delegates to `proxy::execute` until
/// observability/state glue is split in P13-P15. Callers should use this relay
/// entrypoint so ownership can move without touching RelayRun again.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_once(
    state: &ProxyState,
    api_key_id: Option<&str>,
    upstream_api_key: &str,
    key_hint: &str,
    headers: &HeaderMap,
    body: &serde_json::Value,
    client_endpoint: &EndpointType,
    selection: &SelectionResult,
    attempts: &mut Vec<AttemptStats>,
) -> Result<ProxySuccess, ProxyError> {
    crate::proxy::execute::execute_proxy_request(
        state,
        api_key_id,
        upstream_api_key,
        key_hint,
        headers,
        body,
        client_endpoint,
        selection,
        attempts,
    )
    .await
}
