use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use reqwest::Client;
use std::collections::HashMap;

use super::crud::get_channel_by_id;
use super::types::{
    ChannelState, CustomHeader, DetectRequest, DetectResponse, EndpointConfig, EndpointDetection,
    EndpointType, TestChannelRequest, TestChannelResponse,
};
use crate::api::{ApiError, ApiResponse};
use crate::proxy::channel::ChannelInfo;

const TEST_PROMPT: &str = "Hello! Please respond with a brief greeting in one sentence.";

/// 测试渠道 — 直接发送到渠道上游，验证指定 key 能否访问指定模型
pub async fn test_channel(
    State(state): State<ChannelState>,
    Path(id): Path<String>,
    Json(req): Json<TestChannelRequest>,
) -> Result<Json<ApiResponse<TestChannelResponse>>, (StatusCode, Json<ApiError>)> {
    let endpoint_type = parse_protocol(&req.test_protocol)
        .ok_or_else(|| ApiError::bad_request(format!("不支持的测试协议: {}", req.test_protocol)))?;

    let use_stream = req.stream.unwrap_or(false);

    let (body, upstream_path) = if use_stream {
        build_streaming_test_payload(&endpoint_type, &req.model)
    } else {
        build_test_payload(&endpoint_type, &req.model)
    }
    .ok_or_else(|| {
        ApiError::bad_request(format!(
            "协议 {} 不支持{}测试",
            req.test_protocol,
            if use_stream { "流式" } else { "" }
        ))
    })?;

    let channel = get_channel_by_id(&state.pool, &id, state.timezone_offset).await?;
    let api_key = channel
        .api_keys
        .iter()
        .find(|k| k.key == req.api_key && k.enabled)
        .ok_or_else(|| ApiError::bad_request("指定的 API Key 不存在或已禁用"))?;
    let endpoint = channel
        .endpoints
        .iter()
        .find(|e| e.endpoint_type == endpoint_type && e.enabled)
        .ok_or_else(|| ApiError::bad_request(format!("渠道没有启用 {} 端点", req.test_protocol)))?;

    if use_stream {
        let (result, latency_ms, ttft, pt, ct) = probe_endpoint_raw(
            &state.http_client,
            endpoint,
            &api_key.key,
            &body,
            Some(&channel.custom_headers),
            req.user_agent.as_deref(),
        )
        .await;

        match result {
            Ok(content) => Ok(Json(ApiResponse::success(TestChannelResponse {
                success: true,
                message: "模型测试成功（流式）".to_string(),
                latency_ms,
                time_to_first_token_ms: ttft,
                input_prompt: TEST_PROMPT.to_string(),
                output_content: Some(content),
                prompt_tokens: pt,
                completion_tokens: ct,
            }))),
            Err(msg) => Ok(Json(ApiResponse::success(TestChannelResponse {
                success: false,
                message: msg,
                latency_ms,
                time_to_first_token_ms: ttft,
                input_prompt: TEST_PROMPT.to_string(),
                output_content: None,
                prompt_tokens: pt,
                completion_tokens: ct,
            }))),
        }
    } else {
        let start = std::time::Instant::now();

        let mut req_builder = state
            .http_client
            .post(&format!("{}{}", endpoint.base_url.trim_end_matches('/'), upstream_path))
            .header("Content-Type", "application/json")
            .timeout(std::time::Duration::from_secs(30));

        match endpoint_type {
            EndpointType::Anthropic => {
                req_builder = req_builder
                    .header("x-api-key", api_key.key.as_str())
                    .header("anthropic-version", "2023-06-01");
            }
            _ => {
                req_builder =
                    req_builder.header("Authorization", format!("Bearer {}", api_key.key));
            }
        }

        req_builder = inject_custom_headers(req_builder, &channel.custom_headers);

        if let Some(ref ua) = req.user_agent
            && !ua.is_empty()
        {
            req_builder = req_builder.header("User-Agent", ua);
        }

        let resp = req_builder.json(&body).send().await;
        let latency_ms = start.elapsed().as_millis() as u64;

        match resp {
            Ok(resp) => {
                let status = resp.status();
                let resp_text = resp.text().await.unwrap_or_default();

                if !status.is_success() {
                    return Ok(Json(ApiResponse::success(TestChannelResponse {
                        success: false,
                        message: format!("上游返回 HTTP {}: {}", status, resp_text),
                        latency_ms,
                        time_to_first_token_ms: None,
                        input_prompt: TEST_PROMPT.to_string(),
                        output_content: None,
                        prompt_tokens: None,
                        completion_tokens: None,
                    })));
                }

                let resp_body: serde_json::Value =
                    serde_json::from_str(&resp_text).unwrap_or_default();
                if resp_body.get("error").is_some() {
                    let error_msg = resp_body["error"]["message"].as_str().unwrap_or("未知错误");
                    return Ok(Json(ApiResponse::success(TestChannelResponse {
                        success: false,
                        message: format!("模型返回错误: {}", error_msg),
                        latency_ms,
                        time_to_first_token_ms: None,
                        input_prompt: TEST_PROMPT.to_string(),
                        output_content: None,
                        prompt_tokens: None,
                        completion_tokens: None,
                    })));
                }

                let content = extract_test_content(&resp_body, &endpoint_type);
                let (pt, ct) = extract_usage(&resp_body, &endpoint_type);
                Ok(Json(ApiResponse::success(TestChannelResponse {
                    success: true,
                    message: "模型测试成功".to_string(),
                    latency_ms,
                    time_to_first_token_ms: None,
                    input_prompt: TEST_PROMPT.to_string(),
                    output_content: Some(content),
                    prompt_tokens: pt,
                    completion_tokens: ct,
                })))
            }
            Err(e) => Ok(Json(ApiResponse::success(TestChannelResponse {
                success: false,
                message: format!("请求上游失败: {}", e),
                latency_ms,
                time_to_first_token_ms: None,
                input_prompt: TEST_PROMPT.to_string(),
                output_content: None,
                prompt_tokens: None,
                completion_tokens: None,
            }))),
        }
    }
}

/// 检测渠道 quirks：调用上游拿原始响应，分析是否含 `<think/>` 标签或 signature 异常
pub async fn detect_channel_quirks(
    State(state): State<ChannelState>,
    Path(id): Path<String>,
    Json(req): Json<DetectRequest>,
) -> Result<Json<ApiResponse<DetectResponse>>, (StatusCode, Json<ApiError>)> {
    let channel = get_channel_by_id(&state.pool, &id, state.timezone_offset).await?;

    // 选 API key
    let api_key = req
        .api_key
        .as_deref()
        .and_then(|k| channel.api_keys.iter().find(|ak| ak.key == k && ak.enabled))
        .or_else(|| channel.api_keys.iter().find(|ak| ak.enabled))
        .ok_or_else(|| ApiError::bad_request("渠道没有可用的 API Key"))?;

    // 选 model
    let model = req
        .model
        .clone()
        .or_else(|| channel.models.first().cloned())
        .ok_or_else(|| ApiError::bad_request("渠道没有可用模型，请指定 model"))?;

    // 选要测的 endpoints
    let endpoints: Vec<&EndpointConfig> = if let Some(filter) = &req.endpoints {
        channel
            .endpoints
            .iter()
            .filter(|e| e.enabled && filter.iter().any(|t| t == e.endpoint_type.as_str()))
            .collect()
    } else {
        channel.endpoints.iter().filter(|e| e.enabled).collect()
    };

    if endpoints.is_empty() {
        return Err(ApiError::bad_request("没有启用的端点可检测"));
    }

    // 为不同 endpoint 并行构造合适的 thinking-启用请求体
    let endpoint_results: Vec<EndpointDetection> = {
        let probes = endpoints.into_iter().map(|endpoint| {
            let client = &state.http_client;
            let channel = &channel;
            let api_key = api_key.key.clone();
            let model = model.clone();
            async move {
                let (body, upstream_path) = match build_detect_payload(&endpoint.endpoint_type, &model) {
                    Some(v) => v,
                    None => {
                        return EndpointDetection {
                            endpoint: endpoint.endpoint_type.as_str().to_string(),
                            recommendations: HashMap::new(),
                            evidence: format!("协议 {} 不支持检测", endpoint.endpoint_type.as_str()),
                            sample: String::new(),
                        };
                    }
                };
                let (result, _latency, _ttft, _pt, _ct) = probe_endpoint_raw(
                    client,
                    &endpoint,
                    &api_key,
                    &body,
                    Some(&channel.custom_headers),
                    None,
                )
                .await;
                let response_text = match result {
                    Ok(s) => s,
                    Err(e) => format!("[探测调用失败: {}]", e),
                };
                analyze_response(&endpoint.endpoint_type, &response_text)
            }
        });
        futures::future::join_all(probes).await
    };

    // 渠道级合并推荐：任一 endpoint 建议开启 → true
    let mut recommendations: HashMap<String, bool> = HashMap::new();
    for r in &endpoint_results {
        for (k, v) in &r.recommendations {
            if *v {
                recommendations.insert(k.clone(), true);
            }
        }
    }

    Ok(Json(ApiResponse::success(DetectResponse {
        recommendations,
        endpoint_results,
    })))
}

/// 构建测试请求体和上游路径
fn build_test_payload(
    protocol: &EndpointType,
    model: &str,
) -> Option<(serde_json::Value, &'static str)> {
    match protocol {
        EndpointType::OpenAiChat => Some((
            serde_json::json!({
                "model": model,
                "messages": [{"role": "user", "content": TEST_PROMPT}],
                "max_tokens": 100,
                "stream": false
            }),
            "/chat/completions",
        )),
        EndpointType::OpenAiResponse => Some((
            serde_json::json!({
                "model": model,
                "input": TEST_PROMPT,
                "max_output_tokens": 100
            }),
            "/responses",
        )),
        EndpointType::Anthropic => Some((
            serde_json::json!({
                "model": model,
                "messages": [{"role": "user", "content": TEST_PROMPT}],
                "max_tokens": 100
            }),
            "/messages",
        )),
        EndpointType::OpenAiEmbedding => Some((
            serde_json::json!({
                "model": model,
                "input": TEST_PROMPT
            }),
            "/embeddings",
        )),
        EndpointType::OpenAiImages => Some((
            serde_json::json!({
                "model": model,
                "prompt": TEST_PROMPT,
                "n": 1,
                "size": "256x256"
            }),
            "/images/generations",
        )),
        _ => None,
    }
}

/// 构建流式测试请求体
fn build_streaming_test_payload(
    protocol: &EndpointType,
    model: &str,
) -> Option<(serde_json::Value, &'static str)> {
    match protocol {
        EndpointType::OpenAiChat => Some((
            serde_json::json!({
                "model": model,
                "messages": [{"role": "user", "content": TEST_PROMPT}],
                "max_tokens": 100,
                "stream": true
            }),
            "/chat/completions",
        )),
        EndpointType::Anthropic => Some((
            serde_json::json!({
                "model": model,
                "messages": [{"role": "user", "content": TEST_PROMPT}],
                "max_tokens": 100,
                "stream": true
            }),
            "/messages",
        )),
        EndpointType::OpenAiResponse => Some((
            serde_json::json!({
                "model": model,
                "input": TEST_PROMPT,
                "max_output_tokens": 100,
                "stream": true
            }),
            "/responses",
        )),
        _ => None,
    }
}

/// 从响应中提取内容文本
fn extract_test_content(resp_body: &serde_json::Value, endpoint_type: &EndpointType) -> String {
    match endpoint_type {
        EndpointType::OpenAiChat => resp_body["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("(无内容)")
            .to_string(),
        EndpointType::OpenAiResponse => resp_body["output"][0]["content"][0]["text"]
            .as_str()
            .unwrap_or("(无内容)")
            .to_string(),
        EndpointType::Anthropic => resp_body["content"][0]["text"]
            .as_str()
            .unwrap_or("(无内容)")
            .to_string(),
        EndpointType::OpenAiEmbedding => {
            let len = resp_body["data"].as_array().map(|a| a.len()).unwrap_or(0);
            format!("Embedding 返回 {} 条向量数据", len)
        }
        EndpointType::OpenAiImages => {
            let count = resp_body["data"].as_array().map(|a| a.len()).unwrap_or(0);
            format!("图片生成成功，共 {} 张", count)
        }
        _ => "(未知协议)".to_string(),
    }
}

/// 从响应中提取 token 用量
fn extract_usage(
    resp_body: &serde_json::Value,
    endpoint_type: &EndpointType,
) -> (Option<u64>, Option<u64>) {
    let usage = &resp_body["usage"];
    match endpoint_type {
        EndpointType::OpenAiChat => (
            usage["prompt_tokens"].as_u64(),
            usage["completion_tokens"].as_u64(),
        ),
        EndpointType::OpenAiResponse => (
            usage["input_tokens"].as_u64(),
            usage["output_tokens"].as_u64(),
        ),
        EndpointType::Anthropic => (
            usage["input_tokens"].as_u64(),
            usage["output_tokens"].as_u64(),
        ),
        _ => (None, None),
    }
}

/// 解析协议字符串为 EndpointType
fn parse_protocol(protocol: &str) -> Option<EndpointType> {
    serde_json::from_value::<EndpointType>(serde_json::Value::String(protocol.to_string())).ok()
}

/// 注入自定义请求头
fn inject_custom_headers(
    req_builder: reqwest::RequestBuilder,
    headers: &[CustomHeader],
) -> reqwest::RequestBuilder {
    let mut builder = req_builder;
    for header in headers {
        if let Ok(name) = reqwest::header::HeaderName::from_bytes(header.key.as_bytes())
            && let Ok(value) = header.value.parse::<reqwest::header::HeaderValue>()
        {
            builder = builder.header(name, value);
        }
    }
    builder
}

/// 流式测试：发 SSE 请求，消费完整流，返回首 token 时间和完整内容

/// 检测使用的 prompt：包含"请详细分析"等更可能触发 thinking 的内容
const DETECT_PROMPT: &str = "请详细分析 1+1=2 的推理过程，并简短回答。";

/// 共享的端点探测：发流式请求 + 消费完整响应，返回原始响应文本
async fn probe_endpoint_raw(
    client: &Client,
    endpoint: &EndpointConfig,
    api_key: &str,
    body: &serde_json::Value,
    custom_headers: Option<&[CustomHeader]>,
    user_agent: Option<&str>,
) -> (
    Result<String, String>,
    u64,
    Option<u64>,
    Option<u64>,
    Option<u64>,
) {
    let url = format!(
        "{}{}",
        endpoint.base_url.trim_end_matches('/'),
        match endpoint.endpoint_type {
            EndpointType::OpenAiChat => "/chat/completions",
            EndpointType::OpenAiResponse => "/responses",
            EndpointType::Anthropic => "/messages",
            _ => "",
        }
    );

    let mut req_builder = client
        .post(&url)
        .header("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(60));

    match endpoint.endpoint_type {
        EndpointType::Anthropic => {
            req_builder = req_builder
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01");
        }
        _ => {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", api_key));
        }
    }

    if let Some(headers) = custom_headers {
        req_builder = inject_custom_headers(req_builder, headers);
    }

    if let Some(ua) = user_agent
        && !ua.is_empty()
    {
        req_builder = req_builder.header("User-Agent", ua);
    }

    let start = std::time::Instant::now();
    let resp = match req_builder.json(body).send().await {
        Ok(r) => r,
        Err(e) => {
            return (
                Err(format!("请求上游失败: {}", e)),
                start.elapsed().as_millis() as u64,
                None,
                None,
                None,
            );
        }
    };

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return (
            Err(format!("上游返回 HTTP {}: {}", status, text)),
            start.elapsed().as_millis() as u64,
            None,
            None,
            None,
        );
    }

    let bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => {
            return (
                Err(format!("读取响应失败: {}", e)),
                start.elapsed().as_millis() as u64,
                None,
                None,
                None,
            );
        }
    };

    let text = String::from_utf8_lossy(&bytes);
    let mut first_token_ms: Option<u64> = None;
    let mut full_content = String::new();
    let mut prompt_tokens: Option<u64> = None;
    let mut completion_tokens: Option<u64> = None;

    let mut event_type = "";
    for line in text.lines() {
        if let Some(stripped) = line.strip_prefix("event: ") {
            event_type = stripped.trim();
            continue;
        }
        if let Some(data) = line.strip_prefix("data: ") {
            let data = data.trim();
            if data == "[DONE]" {
                continue;
            }
            if first_token_ms.is_none() {
                first_token_ms = Some(start.elapsed().as_millis() as u64);
            }
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                if let Some(v) = json["usage"]["prompt_tokens"].as_u64() {
                    prompt_tokens = Some(v);
                }
                if let Some(v) = json["usage"]["completion_tokens"].as_u64() {
                    completion_tokens = Some(v);
                }
                if let Some(v) = json["usage"]["input_tokens"].as_u64() {
                    prompt_tokens = Some(v);
                }
                if let Some(v) = json["usage"]["output_tokens"].as_u64() {
                    completion_tokens = Some(v);
                }
                if let Some(v) = json["response"]["usage"]["input_tokens"].as_u64() {
                    prompt_tokens = Some(v);
                }
                if let Some(v) = json["response"]["usage"]["output_tokens"].as_u64() {
                    completion_tokens = Some(v);
                }
                if let Some(v) = json["message"]["usage"]["input_tokens"].as_u64() {
                    prompt_tokens = Some(v);
                }
                if let Some(v) = json["message"]["usage"]["output_tokens"].as_u64() {
                    completion_tokens = Some(v);
                }

                let delta = match endpoint.endpoint_type {
                    EndpointType::OpenAiChat => json["choices"][0]["delta"]["content"].as_str(),
                    EndpointType::OpenAiResponse => {
                        if event_type == "response.output_text.delta" {
                            json["delta"].as_str()
                        } else {
                            None
                        }
                    }
                    EndpointType::Anthropic => json["delta"]["text"].as_str(),
                    _ => json["delta"]
                        .as_str()
                        .or_else(|| json["choices"][0]["delta"]["content"].as_str()),
                };
                if let Some(d) = delta {
                    full_content.push_str(d);
                }
            }
        }
    }

    let total_ms = start.elapsed().as_millis() as u64;
    if full_content.is_empty() {
        (
            Err("流式响应无内容".to_string()),
            total_ms,
            first_token_ms,
            prompt_tokens,
            completion_tokens,
        )
    } else {
        (
            Ok(full_content),
            total_ms,
            first_token_ms,
            prompt_tokens,
            completion_tokens,
        )
    }
}

/// 构建检测用的请求体（启用 thinking 模式以触发签名/标签）
fn build_detect_payload(
    protocol: &EndpointType,
    model: &str,
) -> Option<(serde_json::Value, &'static str)> {
    match protocol {
        EndpointType::OpenAiChat => Some((
            serde_json::json!({
                "model": model,
                "messages": [{"role": "user", "content": DETECT_PROMPT}],
                "max_tokens": 200,
                "stream": true
            }),
            "/chat/completions",
        )),
        EndpointType::OpenAiResponse => Some((
            serde_json::json!({
                "model": model,
                "input": DETECT_PROMPT,
                "max_output_tokens": 200,
                "stream": true
            }),
            "/responses",
        )),
        EndpointType::Anthropic => Some((
            serde_json::json!({
                "model": model,
                "messages": [{"role": "user", "content": DETECT_PROMPT}],
                "max_tokens": 200,
                "thinking": {"type": "enabled", "budget_tokens": 1024},
                "stream": true
            }),
            "/messages",
        )),
        _ => None,
    }
}

/// 分析上游响应，检测非标准行为
fn analyze_response(endpoint_type: &EndpointType, response: &str) -> EndpointDetection {
    let mut recommendations: HashMap<String, bool> = HashMap::new();
    let mut evidence_parts: Vec<String> = Vec::new();
    let mut sample = String::new();

    if let Some(tag_start) = response.find("<think>") {
        recommendations.insert("thinking.extract_tags".to_string(), true);
        evidence_parts.push("响应中发现 <think> 标签".to_string());
        if let Some(rel_end) = response[tag_start..].find("</think>") {
            let end = tag_start + rel_end + "</think>".len();
            sample = response[tag_start..end].chars().take(200).collect();
        } else {
            sample = response[tag_start..].chars().take(200).collect();
        }
    }

    if matches!(endpoint_type, EndpointType::Anthropic) {
        let has_content_block_start = response.contains("\"content_block_start\"");
        let has_thinking = response.contains("\"thinking\"");
        let has_signature_field = response.contains("\"signature\":");
        let has_signature_delta = response.contains("\"signature_delta\"");

        if has_content_block_start && has_thinking && has_signature_field && !has_signature_delta {
            recommendations.insert("thinking.fix_signature".to_string(), true);
            evidence_parts
                .push("Anthropic signature 在 content_block_start 内、未见 signature_delta 事件".to_string());
            if let Some(idx) = response.find("\"signature\":") {
                let start = idx + "\"signature\":".len();
                let end = response[start..]
                    .find(|c: char| c == ',' || c == '}' || c == '\n')
                    .unwrap_or(80)
                    .min(80);
                sample = response[idx..start + end].to_string();
            }
        }
    }

    EndpointDetection {
        endpoint: endpoint_type.as_str().to_string(),
        recommendations,
        evidence: evidence_parts.join("；"),
        sample,
    }
}

#[cfg(test)]
mod analyze_response_tests {
    use super::*;

    #[test]
    fn detects_think_tags_in_openai_chat() {
        let resp = r#"data: {"choices":[{"delta":{"content":"<think>hidden</think> visible"}}]}"#;
        let r = analyze_response(&EndpointType::OpenAiChat, resp);
        assert!(r.recommendations.get("thinking.extract_tags").copied().unwrap_or(false));
        assert!(r.evidence.contains("<think>"));
        assert!(r.sample.contains("hidden"));
        assert!(!r.recommendations.contains_key("thinking.fix_signature"));
    }

    #[test]
    fn detects_glm_signature_in_anthropic_start() {
        let resp = r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","signature":"abc123"}}
event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hi"}}
event: content_block_stop
data: {"type":"content_block_stop","index":0}
"#;
        let r = analyze_response(&EndpointType::Anthropic, resp);
        assert!(r.recommendations.get("thinking.fix_signature").copied().unwrap_or(false));
        assert!(r.evidence.contains("content_block_start"));
        assert!(r.sample.contains("abc123"));
    }

    #[test]
    fn no_recommendation_for_standard_anthropic() {
        let resp = r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking"}}
event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"abc"}}
"#;
        let r = analyze_response(&EndpointType::Anthropic, resp);
        assert!(!r.recommendations.contains_key("thinking.fix_signature"));
        assert!(!r.recommendations.contains_key("thinking.extract_tags"));
    }

    #[test]
    fn no_recommendation_for_normal_chat() {
        let resp = r#"data: {"choices":[{"delta":{"content":"hello"}}]}"#;
        let r = analyze_response(&EndpointType::OpenAiChat, resp);
        assert!(r.recommendations.is_empty());
        assert!(r.evidence.is_empty());
    }

    #[test]
    fn does_not_set_fix_signature_for_non_anthropic() {
        // GLM 风格 signature-in-start 只在 anthropic 协议下检测
        let resp = r#"data: {"content_block_start":true,"thinking":true,"signature":"abc"}"#;
        let r = analyze_response(&EndpointType::OpenAiChat, resp);
        assert!(!r.recommendations.contains_key("thinking.fix_signature"));
    }
}

mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_test_payload_for_openai_chat() {
        let (body, path) = build_test_payload(&EndpointType::OpenAiChat, "gpt-4o").unwrap();
        assert_eq!(path, "/chat/completions");
        assert_eq!(body["model"], "gpt-4o");
        assert_eq!(body["stream"], false);
        assert_eq!(body["max_tokens"], 100);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], TEST_PROMPT);
    }

    #[test]
    fn build_test_payload_for_openai_response() {
        let (body, path) = build_test_payload(&EndpointType::OpenAiResponse, "o1").unwrap();
        assert_eq!(path, "/responses");
        assert_eq!(body["model"], "o1");
        assert_eq!(body["input"], TEST_PROMPT);
        assert_eq!(body["max_output_tokens"], 100);
        assert!(body.get("stream").is_none());
    }

    #[test]
    fn build_test_payload_for_anthropic() {
        let (body, path) = build_test_payload(&EndpointType::Anthropic, "claude-sonnet").unwrap();
        assert_eq!(path, "/messages");
        assert_eq!(body["max_tokens"], 100);
        assert_eq!(body["messages"][0]["content"], TEST_PROMPT);
    }

    #[test]
    fn build_test_payload_for_embedding_uses_input_field() {
        let (body, path) =
            build_test_payload(&EndpointType::OpenAiEmbedding, "text-embed").unwrap();
        assert_eq!(path, "/embeddings");
        assert_eq!(body["input"], TEST_PROMPT);
    }

    #[test]
    fn build_test_payload_for_images_uses_prompt_field() {
        let (body, path) = build_test_payload(&EndpointType::OpenAiImages, "dall-e").unwrap();
        assert_eq!(path, "/images/generations");
        assert_eq!(body["prompt"], TEST_PROMPT);
        assert_eq!(body["n"], 1);
    }

    #[test]
    fn build_test_payload_for_gemini_unsupported() {
        assert!(build_test_payload(&EndpointType::Gemini, "gemini-pro").is_none());
    }

    #[test]
    fn build_streaming_test_payload_sets_stream_true() {
        for proto in [
            EndpointType::OpenAiChat,
            EndpointType::Anthropic,
            EndpointType::OpenAiResponse,
        ] {
            let (body, _path) = build_streaming_test_payload(&proto, "m").unwrap();
            assert_eq!(
                body["stream"], true,
                "stream should be true for {:?}",
                proto
            );
        }
    }

    #[test]
    fn build_streaming_test_payload_unsupported_protocols() {
        assert!(build_streaming_test_payload(&EndpointType::OpenAiEmbedding, "m").is_none());
        assert!(build_streaming_test_payload(&EndpointType::OpenAiImages, "m").is_none());
        assert!(build_streaming_test_payload(&EndpointType::Gemini, "m").is_none());
    }

    #[test]
    fn extract_test_content_reads_openai_chat_choices() {
        let body = json!({
            "choices": [{"message": {"content": "hello"}}]
        });
        assert_eq!(
            extract_test_content(&body, &EndpointType::OpenAiChat),
            "hello"
        );
    }

    #[test]
    fn extract_test_content_reads_openai_response_nested_text() {
        let body = json!({
            "output": [{"content": [{"text": "world"}]}]
        });
        assert_eq!(
            extract_test_content(&body, &EndpointType::OpenAiResponse),
            "world"
        );
    }

    #[test]
    fn extract_test_content_reads_anthropic_content_array() {
        let body = json!({
            "content": [{"text": "alpha"}]
        });
        assert_eq!(
            extract_test_content(&body, &EndpointType::Anthropic),
            "alpha"
        );
    }

    #[test]
    fn extract_test_content_falls_back_to_placeholder_on_missing_field() {
        let body = json!({});
        assert_eq!(
            extract_test_content(&body, &EndpointType::OpenAiChat),
            "(无内容)"
        );
        assert_eq!(
            extract_test_content(&body, &EndpointType::Anthropic),
            "(无内容)"
        );
    }

    #[test]
    fn extract_test_content_reports_embedding_count() {
        let body = json!({"data": [{}, {}, {}]});
        assert_eq!(
            extract_test_content(&body, &EndpointType::OpenAiEmbedding),
            "Embedding 返回 3 条向量数据"
        );
    }

    #[test]
    fn extract_test_content_reports_images_count() {
        let body = json!({"data": [{}, {}]});
        assert_eq!(
            extract_test_content(&body, &EndpointType::OpenAiImages),
            "图片生成成功，共 2 张"
        );
    }

    #[test]
    fn extract_test_content_unknown_protocol_returns_marker() {
        let body = json!({});
        assert_eq!(
            extract_test_content(&body, &EndpointType::Gemini),
            "(未知协议)"
        );
    }

    #[test]
    fn extract_usage_openai_chat_reads_prompt_and_completion() {
        let body = json!({
            "usage": {"prompt_tokens": 11, "completion_tokens": 22}
        });
        assert_eq!(
            extract_usage(&body, &EndpointType::OpenAiChat),
            (Some(11), Some(22))
        );
    }

    #[test]
    fn extract_usage_anthropic_reads_input_and_output() {
        let body = json!({
            "usage": {"input_tokens": 7, "output_tokens": 13}
        });
        assert_eq!(
            extract_usage(&body, &EndpointType::Anthropic),
            (Some(7), Some(13))
        );
    }

    #[test]
    fn extract_usage_openai_response_uses_input_output_field_names() {
        let body = json!({
            "usage": {"input_tokens": 3, "output_tokens": 5}
        });
        assert_eq!(
            extract_usage(&body, &EndpointType::OpenAiResponse),
            (Some(3), Some(5))
        );
    }

    #[test]
    fn extract_usage_returns_none_when_usage_missing() {
        let body = json!({});
        assert_eq!(
            extract_usage(&body, &EndpointType::OpenAiChat),
            (None, None)
        );
        assert_eq!(extract_usage(&body, &EndpointType::Anthropic), (None, None));
    }

    #[test]
    fn extract_usage_unsupported_protocol_returns_none() {
        let body = json!({"usage": {"prompt_tokens": 99}});
        assert_eq!(extract_usage(&body, &EndpointType::Gemini), (None, None));
        assert_eq!(
            extract_usage(&body, &EndpointType::OpenAiEmbedding),
            (None, None)
        );
    }

    #[test]
    fn parse_protocol_round_trips_known_strings() {
        assert_eq!(
            parse_protocol("openai_chat"),
            Some(EndpointType::OpenAiChat)
        );
        assert_eq!(
            parse_protocol("openai_response"),
            Some(EndpointType::OpenAiResponse)
        );
        assert_eq!(parse_protocol("anthropic"), Some(EndpointType::Anthropic));
        assert_eq!(parse_protocol("gemini"), Some(EndpointType::Gemini));
        assert_eq!(
            parse_protocol("openai_embedding"),
            Some(EndpointType::OpenAiEmbedding)
        );
        assert_eq!(
            parse_protocol("openai_images"),
            Some(EndpointType::OpenAiImages)
        );
    }

    #[test]
    fn parse_protocol_rejects_unknown_strings() {
        assert_eq!(parse_protocol(""), None);
        assert_eq!(parse_protocol("unknown_protocol"), None);
        assert_eq!(parse_protocol("openai"), None);
    }
}
