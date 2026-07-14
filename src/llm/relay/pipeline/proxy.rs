//! 代理执行：非流式 proxy_request + 流式 proxy_stream。

use axum::http::{HeaderMap, StatusCode};
use std::pin::Pin;

use crate::api::handlers::admin::channels::EndpointType;
use crate::metrics::recorder::save_request_record;
use crate::relay::candidates::build_relay_candidates;
use crate::error::proxy::ProxyError;
use crate::app_state::AppState;

/// 非流式代理请求（支持重试和排队）
pub async fn proxy_request(
    state: &AppState,
    request_id: &str,
    api_key_id: Option<&str>,
    headers: &HeaderMap,
    body: &serde_json::Value,
    client_endpoint: &EndpointType,
) -> Result<crate::app_state::ProxySuccess, ProxyError> {
    let _permit = if let Some(queue) = &state.queue {
        Some(match queue.acquire().await {
            Ok(p) => p,
            Err(e) => {
                // 排队失败也要写一条 usage_logs，避免"CC 已失败但日志缺失"
                let err = ProxyError::RequestError(format!("排队失败: {}", e));
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
                    body["model"].as_str().unwrap_or("unknown"),
                    request_content,
                    None,
                    &[],
                    None,
                    false,
                    user_agent,
                    Some(&err),
                )
                .await;
                return Err(err);
            }
        })
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

        Ok(crate::app_state::ProxySuccess {
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
    state: &AppState,
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
        Some(match queue.acquire().await {
            Ok(p) => p,
            Err(e) => {
                // 排队失败也要写一条 usage_logs，避免"CC 已失败但日志缺失"
                let err = ProxyError::RequestError(format!("排队失败: {}", e));
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
                    body["model"].as_str().unwrap_or("unknown"),
                    request_content,
                    None,
                    &[],
                    None,
                    true,
                    user_agent,
                    Some(&err),
                )
                .await;
                return Err(err);
            }
        })
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
