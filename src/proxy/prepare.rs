use axum::http::HeaderMap;

use super::selection::SelectionResult;
use crate::api::handlers::admin::channels::EndpointType;
use crate::proxy::{ProxyError, ProxyState, get_inbound, get_outbound};
use crate::stats::token_estimator::TokenEstimator;

/// 准备好的代理请求
pub(super) struct PreparedProxyRequest {
    pub(super) body: Vec<u8>,
    pub(super) headers: reqwest::header::HeaderMap,
    pub(super) url: String,
    pub(super) upstream_endpoint: EndpointType,
    pub(super) needs_conversion: bool,
    pub(super) channel_id: String,
    pub(super) model: String,
    pub(super) target_model: String,
}

/// 从响应体提取 usage 数据
pub(super) fn extract_usage(
    body: &serde_json::Value,
    endpoint_type: &EndpointType,
) -> (i32, i32, i32, i32) {
    let usage = &body["usage"];
    match endpoint_type {
        EndpointType::OpenAiChat | EndpointType::OpenAiResponse => {
            let input = usage["prompt_tokens"].as_i64().unwrap_or(0) as i32;
            let output = usage["completion_tokens"].as_i64().unwrap_or(0) as i32;
            let cache_read = usage["prompt_tokens_details"]["cached_tokens"]
                .as_i64()
                .unwrap_or(0) as i32;
            (input, output, cache_read, 0)
        }
        EndpointType::Anthropic => {
            let input = usage["input_tokens"].as_i64().unwrap_or(0) as i32;
            let output = usage["output_tokens"].as_i64().unwrap_or(0) as i32;
            let cache_read = usage["cache_read_input_tokens"].as_i64().unwrap_or(0) as i32;
            let cache_creation = usage["cache_creation_input_tokens"].as_i64().unwrap_or(0) as i32;
            (input, output, cache_read, cache_creation)
        }
        _ => (0, 0, 0, 0),
    }
}

/// 估算 token 数（当上游不返回 usage 时作为兜底）
/// 使用多维度加权估算，区分不同厂商和字符类型
pub(super) fn estimate_tokens(text: &str) -> i32 {
    if text.is_empty() {
        return 0;
    }
    // 默认使用 OpenAI 权重
    let estimator = TokenEstimator::new();
    estimator.estimate(text, &crate::stats::token_estimator::Provider::OpenAI)
}

/// 估算 token 数（指定模型）
#[allow(dead_code)]
pub(super) fn estimate_tokens_for_model(text: &str, model: &str) -> i32 {
    if text.is_empty() {
        return 0;
    }
    let estimator = TokenEstimator::new();
    estimator.estimate_request_tokens(text, model)
}

/// 从请求体提取文本（兼容 OpenAI / Anthropic 格式）
pub(super) fn extract_request_text(body: &serde_json::Value) -> String {
    let mut text = String::new();
    if let Some(sys) = body["system"].as_str() {
        text.push_str(sys);
        text.push(' ');
    }
    if let Some(messages) = body["messages"].as_array() {
        for msg in messages {
            if let Some(content) = msg["content"].as_str() {
                text.push_str(content);
                text.push(' ');
            } else if let Some(parts) = msg["content"].as_array() {
                for part in parts {
                    if let Some(t) = part["text"].as_str() {
                        text.push_str(t);
                        text.push(' ');
                    }
                }
            }
        }
    }
    text
}

/// 从非流式响应体提取文本（兼容 OpenAI / Anthropic 格式）
pub(super) fn extract_response_text(body: &serde_json::Value) -> String {
    let mut text = String::new();
    if let Some(choices) = body["choices"].as_array() {
        for choice in choices {
            if let Some(content) = choice["message"]["content"].as_str() {
                text.push_str(content);
            }
        }
    }
    if let Some(content) = body["content"].as_array() {
        for block in content {
            if let Some(t) = block["text"].as_str() {
                text.push_str(t);
            }
        }
    }
    text
}

/// 选择渠道（支持重试排除）
pub(super) async fn select_channel_for_proxy(
    state: &ProxyState,
    headers: &HeaderMap,
    body: &serde_json::Value,
    client_endpoint: &EndpointType,
    exclude_ids: &[String],
) -> Result<SelectionResult, ProxyError> {
    let model = body["model"].as_str().unwrap_or("unknown");
    let session_hash = headers
        .get("x-session-hash")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .or_else(|| body["session_hash"].as_str().map(|s| s.to_string()));

    tracing::debug!(
        "选择渠道: model={}, endpoint={}, excluded={:?}",
        model,
        client_endpoint.as_str(),
        exclude_ids
    );

    match state
        .select_channel_with_exclude(
            model,
            client_endpoint.clone(),
            session_hash.as_deref(),
            exclude_ids,
        )
        .await
    {
        Ok(sel) => {
            tracing::debug!(
                "选中渠道: channel={} ({}), target_model={}, url={}{}",
                sel.channel.name,
                sel.channel.id,
                sel.target_model,
                sel.endpoint.base_url,
                sel.endpoint.endpoint_type.path()
            );
            Ok(sel)
        }
        Err(e) => {
            tracing::warn!("精确端点匹配失败: {}, 尝试跨协议匹配", e);
            let result = state
                .select_channel_for_model_with_exclude(model, session_hash.as_deref(), exclude_ids)
                .await;
            if let Ok(ref sel) = result {
                tracing::debug!(
                    "跨协议选中: channel={} ({}), endpoint={}",
                    sel.channel.name,
                    sel.channel.id,
                    sel.endpoint.endpoint_type.as_str()
                );
            }
            result
        }
    }
}

/// 准备代理请求（共享逻辑）
pub(super) async fn prepare_proxy_request(
    headers: &HeaderMap,
    body: &serde_json::Value,
    client_endpoint: &EndpointType,
    selection: &SelectionResult,
    api_key: &str,
) -> Result<PreparedProxyRequest, ProxyError> {
    let model = body["model"].as_str().unwrap_or("unknown").to_string();
    let upstream_endpoint = selection.endpoint.endpoint_type.clone();
    let needs_conversion = client_endpoint != &upstream_endpoint;

    let is_stream = body["stream"].as_bool().unwrap_or(false);
    let needs_usage_injection = is_stream
        && matches!(
            upstream_endpoint,
            EndpointType::OpenAiChat | EndpointType::OpenAiResponse
        );

    let mut request_body = if needs_conversion {
        let inbound = get_inbound(client_endpoint);
        let outbound = get_outbound(&upstream_endpoint);
        let body_bytes =
            serde_json::to_vec(body).map_err(|e| ProxyError::TransformError(e.to_string()))?;
        let llm_request = inbound
            .transform_request(&body_bytes, headers)
            .await
            .map_err(|e| ProxyError::TransformError(e.to_string()))?;
        outbound
            .transform_request(&llm_request)
            .map_err(|e| ProxyError::TransformError(e.to_string()))?
    } else {
        let body = body.clone();
        serde_json::to_vec(&body).map_err(|e| ProxyError::TransformError(e.to_string()))?
    };

    // 协议转换和非转换路径统一注入 stream_options
    if needs_usage_injection
        && let Ok(mut req_val) = serde_json::from_slice::<serde_json::Value>(&request_body)
    {
        req_val["stream_options"] = serde_json::json!({"include_usage": true});
        request_body = serde_json::to_vec(&req_val).unwrap_or(request_body);
    }

    // 不应转发到上游的 hop-by-hop / 鉴权 / 连接管理请求头
    static HOP_BY_HOP: &[&str] = &[
        "host",
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
        "content-length",
        "content-type",
        "accept-encoding",
        "authorization",
        "x-api-key",
        "x-forwarded-for",
        "x-forwarded-host",
        "x-forwarded-proto",
        "x-forwarded-port",
        "x-real-ip",
        "forwarded",
        "cookie",
    ];

    let mut reqwest_headers = reqwest::header::HeaderMap::new();
    // 黑名单过滤：透传所有不在 HOP_BY_HOP 中的客户端请求头
    for (key, value) in headers.iter() {
        if HOP_BY_HOP
            .iter()
            .any(|h| key.as_str().eq_ignore_ascii_case(h))
        {
            continue;
        }
        reqwest_headers.insert(key.clone(), value.clone());
    }

    reqwest_headers.insert("Content-Type", "application/json".parse().expect("static value"));

    if needs_conversion {
        get_outbound(&upstream_endpoint).set_auth_header(&mut reqwest_headers, api_key);
    } else {
        match client_endpoint {
            EndpointType::Anthropic => {
                reqwest_headers.insert("x-api-key", api_key.parse().expect("api_key validated at save"));
                reqwest_headers.insert("anthropic-version", "2023-06-01".parse().expect("static value"));
            }
            _ => {
                reqwest_headers.insert(
                    "Authorization",
                    format!("Bearer {}", api_key).parse().expect("api_key validated at save"),
                );
            }
        }
    }

    let url = format!(
        "{}{}",
        selection.endpoint.base_url,
        upstream_endpoint.path()
    );

    for header in &selection.channel.custom_headers {
        if let Ok(name) = reqwest::header::HeaderName::from_bytes(header.key.as_bytes())
            && let Ok(value) = header.value.parse()
        {
            reqwest_headers.insert(name, value);
        }
    }

    Ok(PreparedProxyRequest {
        body: request_body,
        headers: reqwest_headers,
        url,
        upstream_endpoint,
        needs_conversion,
        channel_id: selection.channel.id.clone(),
        model,
        target_model: selection.target_model.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn estimate_tokens_handles_empty() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn estimate_tokens_approximates_english_words() {
        // 使用新的多维度估算，英文单词约 1.02 token/word
        // "a".repeat(30) 是 30 个字母，但被识别为一个单词
        let tokens = estimate_tokens(&"a".repeat(30));
        assert!(tokens >= 1 && tokens <= 5);

        // 多个单词
        let tokens = estimate_tokens("hello world test");
        assert!(tokens >= 3 && tokens <= 6);

        // 单个字符（1.02 向上取整 = 2）
        assert_eq!(estimate_tokens("a"), 2);
    }

    #[test]
    fn extract_usage_for_openai_chat() {
        let body = json!({
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 20,
                "prompt_tokens_details": { "cached_tokens": 5 }
            }
        });
        let (i, o, cr, cc) = extract_usage(&body, &EndpointType::OpenAiChat);
        assert_eq!((i, o, cr, cc), (10, 20, 5, 0));
    }

    #[test]
    fn extract_usage_for_anthropic_includes_cache_creation() {
        let body = json!({
            "usage": {
                "input_tokens": 8,
                "output_tokens": 12,
                "cache_read_input_tokens": 3,
                "cache_creation_input_tokens": 4
            }
        });
        let (i, o, cr, cc) = extract_usage(&body, &EndpointType::Anthropic);
        assert_eq!((i, o, cr, cc), (8, 12, 3, 4));
    }

    #[test]
    fn extract_usage_falls_back_to_zero_on_missing_fields() {
        let body = json!({});
        let (i, o, cr, cc) = extract_usage(&body, &EndpointType::OpenAiChat);
        assert_eq!((i, o, cr, cc), (0, 0, 0, 0));
    }

    #[test]
    fn extract_request_text_collects_system_and_messages() {
        let body = json!({
            "system": "you are helpful",
            "messages": [
                { "content": "hi" },
                { "content": [{ "text": "there" }] }
            ]
        });
        let text = extract_request_text(&body);
        assert!(text.contains("you are helpful"));
        assert!(text.contains("hi"));
        assert!(text.contains("there"));
    }

    #[test]
    fn extract_response_text_reads_openai_choices() {
        let body = json!({
            "choices": [
                { "message": { "content": "hello" } },
                { "message": { "content": "world" } }
            ]
        });
        let text = extract_response_text(&body);
        assert_eq!(text, "helloworld");
    }

    #[test]
    fn extract_response_text_reads_anthropic_content_blocks() {
        let body = json!({
            "content": [
                { "type": "text", "text": "alpha" },
                { "type": "text", "text": "beta" }
            ]
        });
        let text = extract_response_text(&body);
        assert_eq!(text, "alphabeta");
    }
}
