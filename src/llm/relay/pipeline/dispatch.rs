//! 统一派发入口：限流/预算/权限门禁后分流流式与非流式。

use axum::http::HeaderMap;

use crate::domain::channel::EndpointType;
use crate::error::proxy::{ErrorFormat, ProxyError};
use crate::app_state::AppState;
use crate::service::stats::recorder::save_request_record;

use super::access::{check_budget, validate_model_access};
// 经 mod.rs 重导出路径引用（保持 proxy_request/proxy_stream 的重导出被消费，避免 bin unused warning）
use super::{proxy_request, proxy_stream};

/// 统一代理请求入口（供各 handler 调用）
pub async fn handle_proxy_request(
    state: &AppState,
    auth: crate::api::middleware::ApiKeyAuth,
    headers: HeaderMap,
    body: serde_json::Value,
    client_endpoint: &EndpointType,
    error_format: &ErrorFormat,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let request_id = crate::util::id::generate_id();
    let model = body["model"].as_str().unwrap_or("unknown");
    let is_stream = body["stream"].as_bool().unwrap_or(false);
    let api_key_id = Some(auth.key_id.as_str());

    // 速率限制检查（RPM + TPM）—— 高频路径，不写 usage_logs（避免把限流当请求）
    if let Err(retry_after) = state
        .rate_limiter
        .check_rpm(&auth.key_id, auth.rate_limit_rpm)
        .await
    {
        return crate::error::proxy::format_rate_limit_error(retry_after, "RPM", auth.rate_limit_rpm, error_format);
    }
    if let Err(retry_after) = state
        .rate_limiter
        .check_tpm(&auth.key_id, auth.rate_limit_tpm)
        .await
    {
        return crate::error::proxy::format_rate_limit_error(retry_after, "TPM", auth.rate_limit_tpm, error_format);
    }

    // 预算检查（月/日额度）
    if let Err(msg) = check_budget(&state.pool, &auth.key_id).await {
        let e = ProxyError::RequestError(format!("预算超限: {}", msg));
        record_early_failure(
            state, &request_id, api_key_id, model, &headers, &body, is_stream, &e,
        )
        .await;
        return crate::error::proxy::format_budget_error(&msg, error_format);
    }

    // 验证 API Key 是否有权访问目标模型（三段式：403→404→503）
    if let Err(e) =
        validate_model_access(&state.pool, &auth.key_id, model, &auth.allowed_routes).await
    {
        record_early_failure(
            state, &request_id, api_key_id, model, &headers, &body, is_stream, &e,
        )
        .await;
        return crate::error::proxy::format_proxy_error(e, error_format);
    }

    if is_stream {
        match proxy_stream(
            state,
            &request_id,
            api_key_id,
            &headers,
            &body,
            client_endpoint,
        )
        .await
        {
            Ok((status, stream, content_type)) => axum::response::Response::builder()
                .status(status)
                .header("Content-Type", content_type)
                .header("Cache-Control", "no-cache")
                .header("Connection", "keep-alive")
                .header("X-Request-ID", &request_id)
                .body(axum::body::Body::from_stream(stream))
                .expect("static headers + StatusCode from upstream are valid Response inputs")
                .into_response(),
            Err(e) => crate::error::proxy::format_proxy_error(e, error_format),
        }
    } else {
        match proxy_request(
            state,
            &request_id,
            api_key_id,
            &headers,
            &body,
            client_endpoint,
        )
        .await
        {
            Ok(result) => axum::response::Response::builder()
                .status(result.status)
                .header("Content-Type", "application/json")
                .header("X-Request-ID", &request_id)
                .body(axum::body::Body::from(result.body))
                .expect("static Content-Type + StatusCode are valid Response inputs")
                .into_response(),
            Err(e) => crate::error::proxy::format_proxy_error(e, error_format),
        }
    }
}

/// 在 proxy 执行前的早期失败路径（预算/权限检查）补一条 usage_logs。
///
/// 背景：这些路径原本直接返回错误响应而不写 usage_logs，
/// 导致"CC 已经失败但请求日志缺失"现象（参考 axonhub 每次请求必建 Request 实体的设计）。
#[allow(clippy::too_many_arguments)]
async fn record_early_failure(
    state: &AppState,
    request_id: &str,
    api_key_id: Option<&str>,
    model: &str,
    headers: &HeaderMap,
    body: &serde_json::Value,
    is_stream: bool,
    error: &ProxyError,
) {
    let request_content = serde_json::to_string(body).ok();
    let user_agent = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    save_request_record(
        state,
        Some(request_id.to_string()),
        api_key_id,
        None,
        model,
        request_content,
        None,
        &[],
        None,
        is_stream,
        user_agent,
        Some(error),
    )
    .await;
}
