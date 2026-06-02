use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use reqwest::Client;

use super::crud::get_channel_by_id;
use super::types::{ChannelState, CustomHeader, EndpointType, TestChannelRequest, TestChannelResponse};
use crate::api::{ApiError, ApiResponse};

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

    let channel = get_channel_by_id(&state.pool, &id).await?;

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

    let url = format!(
        "{}{}",
        endpoint.base_url.trim_end_matches('/'),
        upstream_path
    );

    if use_stream {
        let (result, latency_ms, ttft, pt, ct) = send_streaming_test(
            &state.http_client,
            &url,
            &body,
            &endpoint_type,
            &api_key.key,
            &channel.custom_headers,
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
            .post(&url)
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
        EndpointType::OpenAiResponse | EndpointType::Anthropic => (
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
async fn send_streaming_test(
    client: &Client,
    url: &str,
    body: &serde_json::Value,
    endpoint_type: &EndpointType,
    api_key: &str,
    custom_headers: &[CustomHeader],
    user_agent: Option<&str>,
) -> (
    Result<String, String>,
    u64,
    Option<u64>,
    Option<u64>,
    Option<u64>,
) {
    let mut req_builder = client
        .post(url)
        .header("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(60));

    match endpoint_type {
        EndpointType::Anthropic => {
            req_builder = req_builder
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01");
        }
        _ => {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", api_key));
        }
    }
    req_builder = inject_custom_headers(req_builder, custom_headers);

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

    for line in text.lines() {
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
                if let Some(v) = json["message"]["usage"]["input_tokens"].as_u64() {
                    prompt_tokens = Some(v);
                }
                let delta = match endpoint_type {
                    EndpointType::OpenAiChat => json["choices"][0]["delta"]["content"].as_str(),
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
