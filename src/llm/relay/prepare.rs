use axum::http::HeaderMap;

use crate::api::handlers::admin::channels::{CustomHeader, EndpointType};
use crate::service::stats::attempt::AttemptStats;
use crate::util::estimator::TokenEstimator;
use crate::relay::converter::{RelayPipeline, RelayPipelineRequest};
use crate::error::proxy::ProxyError;
use crate::llm::plugin::{PluginChain, PluginContext};
use crate::relay::converter::get_outbound;
use crate::scheduler::selector::SelectionResult;

/// 准备好的代理请求
pub(crate) struct PreparedProxyRequest {
    pub(crate) body: Vec<u8>,
    pub(crate) headers: reqwest::header::HeaderMap,
    pub(crate) url: String,
    pub(crate) upstream_endpoint: EndpointType,
    pub(crate) needs_conversion: bool,
    pub(crate) channel_id: String,
    pub(crate) model: String,
    pub(crate) target_model: String,
}

/// 构造一条"失败"的 AttemptStats（send 网络失败 / 上游非 2xx / SSE 流内错误共用）。
///
/// output 为 0（未生成）；input 按请求体估算（客户端确实发送了 input）。流式与非流式
/// 的失败记录共用此构造，避免 executor.rs 与 stream_executor.rs 各写一份导致的"修一个漏一个"。
///
/// 注意：不接收 `&PreparedProxyRequest`，因为 send() 已 move 掉 headers/body，
/// 之后无法整体借用；这里只接收需要的字段（部分 move 后单字段访问是允许的）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn failed_attempt_stats(
    body: &serde_json::Value,
    channel_id: &str,
    target_model: &str,
    upstream_endpoint: &EndpointType,
    needs_conversion: bool,
    latency_ms: i64,
    status_code: u16,
    error_message: String,
    upstream_key_hint: &str,
) -> AttemptStats {
    AttemptStats {
        channel_id: channel_id.to_string(),
        target_model: target_model.to_string(),
        upstream_endpoint: upstream_endpoint.clone(),
        needs_conversion,
        latency_ms,
        status_code,
        // 失败请求客户端确实发送了 input，按请求体估算 input tokens；output 为 0（未生成）
        input_tokens: estimate_request_input_tokens(body, target_model),
        output_tokens: 0,
        cache_read: 0,
        cache_creation: 0,
        cost: None,
        error_message: Some(error_message),
        upstream_key_hint: upstream_key_hint.to_string(),
    }
}

/// 估算请求体的 input tokens（失败请求也记录 input，output 为 0）。
pub(crate) fn estimate_request_input_tokens(body: &serde_json::Value, model: &str) -> i32 {
    estimate_tokens_for_model(&extract_request_text(body), model)
}

/// 把自定义请求头注入到上游请求（同名 insert 覆盖）。
fn inject_custom_headers(headers: &mut reqwest::header::HeaderMap, custom: &[CustomHeader]) {
    for header in custom {
        if let Ok(name) = reqwest::header::HeaderName::from_bytes(header.key.as_bytes())
            && let Ok(value) = header.value.parse()
        {
            headers.insert(name, value);
        }
    }
}

/// 从响应体提取 usage 数据
pub(crate) fn extract_usage(
    body: &serde_json::Value,
    endpoint_type: &EndpointType,
) -> (i32, i32, i32, i32) {
    let usage = &body["usage"];
    match endpoint_type {
        EndpointType::OpenAiChat => {
            let input = usage["prompt_tokens"].as_i64().unwrap_or(0) as i32;
            let output = usage["completion_tokens"].as_i64().unwrap_or(0) as i32;
            let cache_read = usage["prompt_tokens_details"]["cached_tokens"]
                .as_i64()
                .unwrap_or(0) as i32;
            (input, output, cache_read, 0)
        }
        EndpointType::OpenAiResponse => {
            let input = usage["input_tokens"].as_i64().unwrap_or(0) as i32;
            let output = usage["output_tokens"].as_i64().unwrap_or(0) as i32;
            let cache_read = usage["input_tokens_details"]["cached_tokens"]
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
pub(crate) fn estimate_tokens(text: &str) -> i32 {
    if text.is_empty() {
        return 0;
    }
    // 默认使用 OpenAI 权重
    let estimator = TokenEstimator::new();
    estimator.estimate(text, &crate::util::estimator::Provider::OpenAI)
}

/// 估算 token 数（指定模型）
pub(crate) fn estimate_tokens_for_model(text: &str, model: &str) -> i32 {
    if text.is_empty() {
        return 0;
    }
    let estimator = TokenEstimator::new();
    estimator.estimate_request_tokens(text, model)
}

/// 从请求体提取文本（兼容 OpenAI Chat / Anthropic / OpenAI Responses 格式）
pub(crate) fn extract_request_text(body: &serde_json::Value) -> String {
    let mut text = String::new();
    if let Some(sys) = body["system"].as_str() {
        text.push_str(sys);
        text.push(' ');
    }
    // OpenAI Chat / Anthropic: messages[]
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
    // OpenAI Responses: input (string or array)
    if let Some(input) = body.get("input") {
        if let Some(s) = input.as_str() {
            text.push_str(s);
            text.push(' ');
        } else if let Some(input_array) = input.as_array() {
            for item in input_array {
                // 数组项可能是 { "role": "...", "content": "..." } 或纯文本
                if let Some(content) = item["content"].as_str() {
                    text.push_str(content);
                    text.push(' ');
                } else if let Some(parts) = item["content"].as_array() {
                    for part in parts {
                        if let Some(t) = part["text"].as_str() {
                            text.push_str(t);
                            text.push(' ');
                        }
                    }
                } else if let Some(s) = item.as_str() {
                    text.push_str(s);
                    text.push(' ');
                }
            }
        }
    }
    text
}

/// 从非流式响应体提取文本（兼容 OpenAI Chat / Anthropic / OpenAI Responses 格式）
pub(crate) fn extract_response_text(body: &serde_json::Value) -> String {
    let mut text = String::new();
    // OpenAI Chat: choices[].message.content
    if let Some(choices) = body["choices"].as_array() {
        for choice in choices {
            if let Some(content) = choice["message"]["content"].as_str() {
                text.push_str(content);
            }
        }
    }
    // Anthropic: content[].text
    if let Some(content) = body["content"].as_array() {
        for block in content {
            if let Some(t) = block["text"].as_str() {
                text.push_str(t);
            }
        }
    }
    // OpenAI Responses: output[].content[].text
    if let Some(output) = body["output"].as_array() {
        for item in output {
            if let Some(content) = item["content"].as_array() {
                for part in content {
                    if let Some(t) = part["text"].as_str() {
                        text.push_str(t);
                    }
                }
            }
        }
    }
    text
}

/// 准备代理请求（共享逻辑）
pub(crate) async fn prepare_proxy_request(
    headers: &HeaderMap,
    body: &serde_json::Value,
    client_endpoint: &EndpointType,
    selection: &SelectionResult,
    api_key: &str,
    plugin_chain: &PluginChain,
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

    let prepared_pipeline = RelayPipeline::prepare_request_async(RelayPipelineRequest {
        client_endpoint: client_endpoint.clone(),
        upstream_endpoint: upstream_endpoint.clone(),
        requested_model: model.clone(),
        upstream_model: selection.target_model.clone(),
        body: body.clone(),
    })
    .await
    .map_err(|e| ProxyError::TransformError(e.to_string()))?;
    let mut body_value = prepared_pipeline.body;

    // Step C：请求插件链改写（cch / tracking / cache_key）
    let ctx = PluginContext {
        upstream_endpoint: upstream_endpoint.clone(),
        channel_id: selection.channel.id.clone(),
        host_key: selection.channel.key_hint(api_key),
        client_name: headers
            .get("user-agent")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string()),
    };
    body_value = plugin_chain.apply_request(body_value, &ctx).await?;

    // 协议转换和非转换路径统一注入 stream_options（直接改 Value，单次序列化）
    if needs_usage_injection {
        body_value["stream_options"] = serde_json::json!({"include_usage": true});
    }

    let request_body = serde_json::to_vec(&body_value)
        .map_err(|e| ProxyError::TransformError(e.to_string()))?;

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

    reqwest_headers.insert(
        "Content-Type",
        "application/json".parse().expect("static value"),
    );

    if needs_conversion {
        get_outbound(&upstream_endpoint).set_auth_header(&mut reqwest_headers, api_key);
    } else {
        match client_endpoint {
            EndpointType::Anthropic => {
                reqwest_headers.insert(
                    "x-api-key",
                    api_key.parse().expect("api_key validated at save"),
                );
                reqwest_headers.insert(
                    "anthropic-version",
                    "2023-06-01".parse().expect("static value"),
                );
            }
            _ => {
                reqwest_headers.insert(
                    "Authorization",
                    format!("Bearer {}", api_key)
                        .parse()
                        .expect("api_key validated at save"),
                );
            }
        }
    }

    let url = format!(
        "{}{}",
        selection.endpoint.base_url,
        upstream_endpoint.path()
    );

    // 端点级自定义请求头（insert 覆盖客户端，用于按协议配不同 User-Agent 等）
    inject_custom_headers(&mut reqwest_headers, &selection.endpoint.headers);

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
        assert!((1..=5).contains(&tokens));

        // 多个单词
        let tokens = estimate_tokens("hello world test");
        assert!((3..=6).contains(&tokens));

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
    fn extract_usage_for_openai_response() {
        let body = json!({
            "usage": {
                "input_tokens": 15,
                "output_tokens": 25,
                "input_tokens_details": { "cached_tokens": 7 }
            }
        });
        let (i, o, cr, cc) = extract_usage(&body, &EndpointType::OpenAiResponse);
        assert_eq!((i, o, cr, cc), (15, 25, 7, 0));
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
    fn extract_request_text_reads_responses_input_string() {
        let body = json!({
            "model": "gpt-4o",
            "input": "Hello world"
        });
        let text = extract_request_text(&body);
        assert!(text.contains("Hello world"));
    }

    #[test]
    fn extract_request_text_reads_responses_input_array() {
        let body = json!({
            "model": "gpt-4o",
            "input": [
                { "role": "user", "content": [{ "type": "input_text", "text": "Hi there" }] },
                { "role": "user", "content": "plain text" }
            ]
        });
        let text = extract_request_text(&body);
        assert!(text.contains("Hi there"));
        assert!(text.contains("plain text"));
    }

    #[tokio::test]
    async fn prepare_proxy_request_conversion_uses_selection_target_model() {
        let body = json!({
            "model": "alias-claude",
            "messages": [
                {"role": "system", "content": "Be concise"},
                {"role": "user", "content": "hello"}
            ],
            "max_completion_tokens": 123,
            "stream": false
        });
        let selection = SelectionResult {
            channel: crate::domain::channel::ChannelInfo {
                id: "ch-1".into(),
                name: "anthropic".into(),
                api_keys: vec![],
                endpoints: vec![],
                models: vec!["alias-claude".into()],
                timeout_secs: 300,
                max_concurrency: 0,
                failure_threshold: 3,
                blacklist_minutes: 10,
            },
            target_model: "claude-3-5-sonnet".into(),
            endpoint: crate::api::handlers::admin::channels::EndpointConfig {
                endpoint_type: EndpointType::Anthropic,
                base_url: "https://api.anthropic.com".into(),
                enabled: true,
                headers: vec![],
            },
            route_id: None,
        };

        let prepared = prepare_proxy_request(
            &HeaderMap::new(),
            &body,
            &EndpointType::OpenAiChat,
            &selection,
            "sk-test",
            &PluginChain::new_empty(),
        )
        .await
        .expect("prepare succeeds");
        let upstream_body: serde_json::Value = serde_json::from_slice(&prepared.body).unwrap();

        assert!(prepared.needs_conversion);
        assert_eq!(upstream_body["model"], "claude-3-5-sonnet");
        assert_eq!(upstream_body["system"], "Be concise");
        assert_eq!(upstream_body["messages"][0]["content"], "hello");
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

    #[test]
    fn extract_response_text_reads_openai_response_output() {
        let body = json!({
            "output": [
                {
                    "type": "message",
                    "content": [
                        { "type": "output_text", "text": "Hello" },
                        { "type": "output_text", "text": " World" }
                    ]
                },
                {
                    "type": "message",
                    "content": [
                        { "type": "output_text", "text": "!" }
                    ]
                }
            ]
        });
        let text = extract_response_text(&body);
        assert_eq!(text, "Hello World!");
    }
}
