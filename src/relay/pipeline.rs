use axum::http::{HeaderMap, StatusCode};
use sqlx::SqlitePool;
use std::pin::Pin;

use crate::api::handlers::admin::channels::EndpointType;
use crate::metrics::recorder::redaction::sanitize_json_content;
use crate::metrics::recorder::save_request_record;
use crate::relay::candidates::build_relay_candidates;
use crate::relay::error::{ErrorFormat, ProxyError};
use crate::relay::state::ProxyState;

/// 验证字符串可作为 HTTP header value
/// 用于在保存上游 API Key 时一次性拦截含 CRLF / 控制字符的输入，
/// 避免转发时 `HeaderValue::from_str(...).unwrap()` panic。
pub(crate) fn validate_header_value(s: &str) -> Result<(), String> {
    reqwest::header::HeaderValue::from_str(s)
        .map(|_| ())
        .map_err(|e| format!("含非法 header 字符 ({e})"))
}

/// 检查 API Key 的预算额度（月/日）
async fn check_budget(pool: &SqlitePool, key_id: &str) -> Result<(), String> {
    let limit: Option<(f64, f64)> = sqlx::query_as(
        "SELECT monthly_limit_usd, daily_limit_usd FROM budget_limits WHERE api_key_id = ? AND enabled = 1",
    )
    .bind(key_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("查询预算失败: {}", e))?;

    let Some((monthly_limit, daily_limit)) = limit else {
        return Ok(()); // 无预算限制
    };

    // 查询累计消费
    let (monthly_cost, daily_cost): (f64, f64) = sqlx::query_as(
        r#"SELECT
            COALESCE(SUM(COALESCE(cost, 0)), 0),
            COALESCE(SUM(CASE WHEN date(created_at) = date('now') THEN COALESCE(cost, 0) ELSE 0 END), 0)
        FROM usage_logs WHERE api_key_id = ?"#,
    )
    .bind(key_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("查询消费失败: {}", e))?;

    if daily_limit > 0.0 && daily_cost >= daily_limit {
        return Err(format!(
            "日预算已耗尽: ${:.2}/${:.2}",
            daily_cost, daily_limit
        ));
    }
    if monthly_limit > 0.0 && monthly_cost >= monthly_limit {
        return Err(format!(
            "月预算已耗尽: ${:.2}/${:.2}",
            monthly_cost, monthly_limit
        ));
    }

    Ok(())
}

/// 验证 API Key 是否有权访问目标模型（三段式：403→404→503）
async fn validate_model_access(
    pool: &SqlitePool,
    key_id: &str,
    model: &str,
    allowed_groups: &str,
) -> Result<(), ProxyError> {
    // === 第一关：检查 allowed_groups / supported_models ===
    // allowed_groups 非空时，检查请求的 model 是否在允许的分组列表中
    if !allowed_groups.is_empty() {
        let allowed: Vec<&str> = allowed_groups
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if !allowed.is_empty() && !allowed.contains(&model) {
            return Err(ProxyError::ModelNotSupported(format!(
                "API Key 无权访问模型: {}",
                model
            )));
        }
    } else {
        // allowed_groups 为空时回退到 supported_models（兼容旧逻辑）
        let supported = sqlx::query_scalar::<_, String>(
            "SELECT supported_models FROM api_keys WHERE id = ? AND enabled = 1",
        )
        .bind(key_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        if let Some(models_str) = supported
            && !models_str.is_empty()
        {
            let allowed =
                crate::api::handlers::admin::api_keys::parse_supported_models(&models_str);
            if !allowed.iter().any(|m| m == model) {
                return Err(ProxyError::ModelNotSupported(format!(
                    "API Key 无权访问模型: {}",
                    model
                )));
            }
        }
    }

    // === 第二关：检查分组是否存在 ===
    // 仅在 allowed_groups 非空时严格检查（Octopus 策略：明确指定了分组才校验）
    // allowed_groups 为空时跳过，由后续 Relay candidate 构建检查渠道可用性
    if !allowed_groups.is_empty() {
        let group_exists: bool = sqlx::query_scalar::<_, i32>(
            "SELECT COUNT(*) FROM groups WHERE name = ? AND enabled = 1",
        )
        .bind(model)
        .fetch_one(pool)
        .await
        .map(|c| c > 0)
        .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        if !group_exists {
            return Err(ProxyError::ModelNotFound(format!("模型不存在: {}", model)));
        }
    }

    // === 第三关：分组内是否有可用渠道（延迟到 Relay candidate 构建时检查）===
    Ok(())
}

/// 非流式代理请求（支持重试和排队）
pub async fn proxy_request(
    state: &ProxyState,
    request_id: &str,
    api_key_id: Option<&str>,
    headers: &HeaderMap,
    body: &serde_json::Value,
    client_endpoint: &EndpointType,
) -> Result<crate::relay::state::ProxySuccess, ProxyError> {
    let _permit = if let Some(queue) = &state.queue {
        Some(
            queue
                .acquire()
                .await
                .map_err(|e| ProxyError::RequestError(format!("排队失败: {}", e)))?,
        )
    } else {
        None
    };

    let model = body["model"].as_str().unwrap_or("unknown").to_string();
    let request_content = serde_json::to_string(&body)
        .ok()
        .map(|c| sanitize_json_content(&c));
    let user_agent = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let session_hash = headers
        .get("x-session-hash")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let sticky_channel_id = if let Some(hash) = session_hash.as_deref() {
        state.lb_state.get_sticky_session(hash).await
    } else {
        None
    };

    // 1. 构建候选列表（sticky + scored group items）
    let candidates =
        match build_relay_candidates(state, &model, client_endpoint, session_hash.as_deref()).await
        {
            Ok(c) => c,
            Err(e) => {
                save_request_record(
                    state,
                    Some(request_id.to_string()),
                    api_key_id,
                    None,
                    &model,
                    request_content,
                    None,
                    &[],
                    None,
                    false,
                    user_agent,
                    Some(&e),
                )
                .await;
                return Err(e);
            }
        };

    // 2. 创建 executor + RelayRun
    let executor = crate::relay::executor::ProxyRelayExecutor::new(
        state.clone(),
        headers.clone(),
        body.clone(),
        client_endpoint.clone(),
        api_key_id.map(|s| s.to_string()),
    );

    let capacity = state.lb_state.capacity_manager();
    let run = crate::relay::run::RelayRun::new(capacity, executor.clone());
    let relay_request = crate::relay::run::RelayRequest::new(&model);

    // 3. 执行
    let candidate_snapshot = candidates.clone();
    let outcome = run.execute(relay_request, candidates).await;
    let attempt_stats = executor.take_attempt_stats();

    // 4. 记录日志 + 返回结果
    if let Some(response_body) = outcome.response {
        if let Some(channel_id) = outcome.selected_channel_id.as_deref() {
            let selected_was_sticky = candidate_snapshot
                .iter()
                .any(|c| c.channel_id == channel_id && c.sticky);
            state.lb_state.record_scheduler_selection(
                channel_id,
                selected_was_sticky,
                sticky_channel_id.as_deref(),
            );
            if let Some(hash) = session_hash.as_deref() {
                state.lb_state.set_sticky_session(hash, channel_id).await;
            }
        }
        save_request_record(
            state,
            Some(request_id.to_string()),
            api_key_id,
            None,
            &model,
            request_content,
            Some(response_body.clone()),
            &attempt_stats,
            None,
            false,
            user_agent,
            None,
        )
        .await;

        Ok(crate::relay::state::ProxySuccess {
            status: StatusCode::OK,
            body: response_body.into_bytes(),
        })
    } else {
        let error = ProxyError::from_relay_outcome(&outcome);
        save_request_record(
            state,
            Some(request_id.to_string()),
            api_key_id,
            None,
            &model,
            request_content,
            None,
            &attempt_stats,
            None,
            false,
            user_agent,
            Some(&error),
        )
        .await;
        Err(error)
    }
}

/// 流式代理请求（支持重试和排队）
pub async fn proxy_stream(
    state: &ProxyState,
    request_id: &str,
    api_key_id: Option<&str>,
    headers: &HeaderMap,
    body: &serde_json::Value,
    client_endpoint: &EndpointType,
) -> Result<
    (
        StatusCode,
        Pin<
            Box<
                dyn futures::Stream<Item = Result<axum::body::Bytes, std::convert::Infallible>>
                    + Send
                    + 'static,
            >,
        >,
        String,
    ),
    ProxyError,
> {
    let queue_permit = if let Some(queue) = &state.queue {
        Some(
            queue
                .acquire()
                .await
                .map_err(|e| ProxyError::RequestError(format!("排队失败: {}", e)))?,
        )
    } else {
        None
    };

    let model = body["model"].as_str().unwrap_or("unknown").to_string();
    let request_content = serde_json::to_string(&body).ok();
    let user_agent = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let session_hash = headers
        .get("x-session-hash")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let sticky_channel_id = if let Some(hash) = session_hash.as_deref() {
        state.lb_state.get_sticky_session(hash).await
    } else {
        None
    };

    let candidates =
        match build_relay_candidates(state, &model, client_endpoint, session_hash.as_deref()).await
        {
            Ok(c) => c,
            Err(e) => {
                save_request_record(
                    state,
                    Some(request_id.to_string()),
                    api_key_id,
                    None,
                    &model,
                    request_content,
                    None,
                    &[],
                    None,
                    true,
                    user_agent,
                    Some(&e),
                )
                .await;
                return Err(e);
            }
        };

    let executor = crate::relay::stream_executor::ProxyStreamRelayExecutor::new(
        state.clone(),
        request_id.to_string(),
        headers.clone(),
        body.clone(),
        client_endpoint.clone(),
        api_key_id.map(|s| s.to_string()),
        queue_permit,
    );
    let run =
        crate::relay::run::RelayStreamRun::new(state.lb_state.capacity_manager(), executor.clone());
    let candidate_snapshot = candidates.clone();
    let outcome = run
        .execute(crate::relay::run::RelayRequest::new(&model), candidates)
        .await;

    if let Some(success) = outcome.response {
        if let Some(channel_id) = outcome.selected_channel_id.as_deref() {
            let selected_was_sticky = candidate_snapshot
                .iter()
                .any(|c| c.channel_id == channel_id && c.sticky);
            state.lb_state.record_scheduler_selection(
                channel_id,
                selected_was_sticky,
                sticky_channel_id.as_deref(),
            );
            if let Some(hash) = session_hash.as_deref() {
                state.lb_state.set_sticky_session(hash, channel_id).await;
            }
        }
        return Ok((success.status, success.stream, success.content_type));
    }

    let attempt_stats = executor.take_attempt_stats();
    let error = ProxyError::from_relay_stream_outcome(&outcome);
    save_request_record(
        state,
        Some(request_id.to_string()),
        api_key_id,
        None,
        &model,
        request_content,
        None,
        &attempt_stats,
        None,
        true,
        user_agent,
        Some(&error),
    )
    .await;
    Err(error)
}

/// 统一代理请求入口（供各 handler 调用）
pub async fn handle_proxy_request(
    state: &ProxyState,
    auth: crate::api::middleware::ApiKeyAuth,
    headers: HeaderMap,
    body: serde_json::Value,
    client_endpoint: &EndpointType,
    error_format: &ErrorFormat,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let request_id = crate::api::response::generate_id();
    let model = body["model"].as_str().unwrap_or("unknown");
    let is_stream = body["stream"].as_bool().unwrap_or(false);
    let api_key_id = Some(auth.key_id.as_str());

    // 速率限制检查（RPM + TPM）
    if let Err(retry_after) = state
        .rate_limiter
        .check_rpm(&auth.key_id, auth.rate_limit_rpm)
        .await
    {
        return crate::relay::error::format_rate_limit_error(retry_after, "RPM", auth.rate_limit_rpm, error_format);
    }
    if let Err(retry_after) = state
        .rate_limiter
        .check_tpm(&auth.key_id, auth.rate_limit_tpm)
        .await
    {
        return crate::relay::error::format_rate_limit_error(retry_after, "TPM", auth.rate_limit_tpm, error_format);
    }

    // 预算检查（月/日额度）
    if let Err(msg) = check_budget(&state.pool, &auth.key_id).await {
        return crate::relay::error::format_budget_error(&msg, error_format);
    }

    // 验证 API Key 是否有权访问目标模型（三段式：403→404→503）
    if let Err(e) =
        validate_model_access(&state.pool, &auth.key_id, model, &auth.allowed_groups).await
    {
        return crate::relay::error::format_proxy_error(e, error_format);
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
            Err(e) => crate::relay::error::format_proxy_error(e, error_format),
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
            Err(e) => crate::relay::error::format_proxy_error(e, error_format),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_header_value_accepts_normal_and_rejects_crlf() {
        assert!(validate_header_value("sk-abc.123_OK-9").is_ok());
        assert!(validate_header_value("sk-abc/def+ghi=").is_ok());
        assert!(validate_header_value("sk-abc").is_ok());
        assert!(validate_header_value("sk-abc\r\nfoo").is_err());
        assert!(validate_header_value("sk-abc\0").is_err());
        assert!(validate_header_value("sk-abc\x7f").is_err());
    }
}
