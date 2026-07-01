use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use reqwest::Client;

use super::crud::get_channel_by_id;
use super::types::{
    ChannelState, CustomHeader, EndpointConfig, EndpointType, TestEndpointRequest,
    TestEndpointResponse,
};
use crate::error::app::{ApiError, ApiResponse};

/// 端点测试：对指定端点发流式探测请求，返回连通性 + 思维链诊断（per-endpoint，不合并）。
///
/// 一次请求同时拿：
/// - 连通性（success/latency/ttft/tokens）
/// - 思维链诊断（响应是否含 `<think>` 标签 + 样本）—— 只检测不应用
pub async fn test_endpoint(
    State(state): State<ChannelState>,
    Path(id): Path<String>,
    Json(req): Json<TestEndpointRequest>,
) -> Result<Json<ApiResponse<TestEndpointResponse>>, (StatusCode, Json<ApiError>)> {
    let endpoint_type = parse_protocol(&req.endpoint_type)
        .ok_or_else(|| ApiError::bad_request(format!("不支持的端点类型: {}", req.endpoint_type)))?;

    let (body, _upstream_path) = build_probe_payload(&endpoint_type, &req.model).ok_or_else(|| {
        ApiError::bad_request(format!("端点类型 {} 不支持测试", req.endpoint_type))
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
        .ok_or_else(|| ApiError::bad_request(format!("渠道没有启用的 {} 端点", req.endpoint_type)))?;

    let (result, latency_ms, ttft, pt, ct) = probe_endpoint_raw(
        &state.http_client,
        endpoint,
        &api_key.key,
        &body,
        req.user_agent.as_deref(),
    )
    .await;

    let (success, content, error) = match result {
        Ok(c) => (true, Some(c), None),
        Err(msg) => (false, None, Some(msg)),
    };
    // 思维链诊断：成功时在响应内容里检测 <think> 标签（协议无关字符串匹配）
    let (thinking_detected, thinking_sample) = if let Some(ref text) = content {
        detect_thinking(text)
    } else {
        (false, None)
    };

    Ok(Json(ApiResponse::success(TestEndpointResponse {
        success,
        latency_ms,
        time_to_first_token_ms: ttft,
        output_content: content,
        error,
        prompt_tokens: pt,
        completion_tokens: ct,
        thinking_detected,
        thinking_sample,
    })))
}

/// 从响应文本检测 `<think>` 标签：返回 (detected, sample)。
/// sample 截取首个 `<think>...</think>` 块（最多 200 字符）；未闭合则截到 200 字符。
fn detect_thinking(response: &str) -> (bool, Option<String>) {
    let Some(tag_start) = response.find("<think>") else {
        return (false, None);
    };
    let sample = if let Some(rel_end) = response[tag_start..].find("</think>") {
        let end = tag_start + rel_end + "</think>".len();
        response[tag_start..end].chars().take(200).collect()
    } else {
        response[tag_start..].chars().take(200).collect()
    };
    (true, Some(sample))
}

/// 构建端点探测请求体（流式 + 带 thinking 参数，触发上游返回思维链以供诊断）。
fn build_probe_payload(
    protocol: &EndpointType,
    model: &str,
) -> Option<(serde_json::Value, &'static str)> {
    match protocol {
        EndpointType::OpenAiChat => Some((
            serde_json::json!({
                "model": model,
                "messages": [{"role": "user", "content": DETECT_PROMPT}],
                "max_tokens": 200,
                "stream": true,
                // OpenAI 标准 reasoning 控制（o-series；非 reasoning 模型忽略，不报错）
                "reasoning_effort": "medium"
            }),
            "/chat/completions",
        )),
        EndpointType::OpenAiResponse => Some((
            serde_json::json!({
                "model": model,
                "input": DETECT_PROMPT,
                "max_output_tokens": 200,
                "stream": true,
                // Responses API 标准 reasoning 控制
                "reasoning": {"effort": "medium"}
            }),
            "/responses",
        )),
        EndpointType::Anthropic => Some((
            serde_json::json!({
                "model": model,
                "messages": [{"role": "user", "content": DETECT_PROMPT}],
                // Anthropic 规则：budget_tokens 必须 < max_tokens（原代码 max 200 < budget 1024 违规）
                "max_tokens": 2048,
                "thinking": {"type": "enabled", "budget_tokens": 1024},
                "stream": true
            }),
            "/messages",
        )),
        _ => None,
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

/// 探测 prompt：包含"请详细分析"等更可能触发 thinking 的内容
const DETECT_PROMPT: &str = "请详细分析 1+1=2 的推理过程，并简短回答。";

/// 共享的端点探测：发流式请求 + 增量消费响应，返回完整响应内容 + 计时/token。
///
/// **增量读**：逐 chunk 累积，遇 `[DONE]` 立即停止，不等连接关闭
/// （修并发测试时 SSE keep-alive 超时误判：原 `resp.bytes().await` 等服务器关连接才返回）。
async fn probe_endpoint_raw(
    client: &Client,
    endpoint: &EndpointConfig,
    api_key: &str,
    body: &serde_json::Value,
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

    req_builder = inject_custom_headers(req_builder, &endpoint.headers);

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

    // 增量读：逐 chunk 累积，遇 [DONE] 立即停止（不等连接关闭）
    use futures::StreamExt;
    let mut stream = resp.bytes_stream();
    let mut text_buf = String::new();
    let mut read_err: Option<String> = None;
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(bytes) => {
                text_buf.push_str(&String::from_utf8_lossy(&bytes));
                if text_buf.contains("[DONE]") {
                    break;
                }
            }
            Err(e) => {
                read_err = Some(format!("读取响应失败: {}", e));
                break;
            }
        }
    }
    if let Some(msg) = read_err {
        return (
            Err(msg),
            start.elapsed().as_millis() as u64,
            None,
            None,
            None,
        );
    }

    // 解析累积的响应文本：提取 content / ttft / usage
    let mut first_token_ms: Option<u64> = None;
    let mut full_content = String::new();
    let mut prompt_tokens: Option<u64> = None;
    let mut completion_tokens: Option<u64> = None;
    let mut event_type = "";

    for line in text_buf.lines() {
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

#[cfg(test)]
mod tests {
    use super::*;

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

    /// build_probe_payload 所有支持的协议 stream:true + 各自标准的 reasoning 控制参数
    #[test]
    fn build_probe_payload_sets_stream_and_reasoning() {
        // OpenAiChat: stream + reasoning_effort（OpenAI 标准）
        let (chat, _) = build_probe_payload(&EndpointType::OpenAiChat, "m").unwrap();
        assert_eq!(chat["stream"], true);
        assert_eq!(chat["reasoning_effort"], "medium");

        // OpenAiResponse: stream + reasoning（Responses API 标准）
        let (resp, _) = build_probe_payload(&EndpointType::OpenAiResponse, "m").unwrap();
        assert_eq!(resp["stream"], true);
        assert_eq!(resp["reasoning"]["effort"], "medium");

        // Anthropic: stream + thinking（Anthropic 标准）
        let (anth, _) = build_probe_payload(&EndpointType::Anthropic, "m").unwrap();
        assert_eq!(anth["stream"], true);
        assert!(anth.get("thinking").is_some());
    }

    /// build_probe_payload 不支持 embedding/images/gemini
    #[test]
    fn build_probe_payload_unsupported_protocols() {
        assert!(build_probe_payload(&EndpointType::OpenAiEmbedding, "m").is_none());
        assert!(build_probe_payload(&EndpointType::OpenAiImages, "m").is_none());
        assert!(build_probe_payload(&EndpointType::Gemini, "m").is_none());
    }

    /// Anthropic 的 budget_tokens 必须 < max_tokens（原 build_detect_payload 违规已修）
    #[test]
    fn build_probe_payload_anthropic_budget_within_max_tokens() {
        let (body, _) = build_probe_payload(&EndpointType::Anthropic, "claude").unwrap();
        let max_tokens = body["max_tokens"].as_u64().unwrap();
        let budget = body["thinking"]["budget_tokens"].as_u64().unwrap();
        assert!(
            budget < max_tokens,
            "budget_tokens ({}) must be < max_tokens ({})",
            budget,
            max_tokens
        );
    }

    /// detect_thinking：含 <think> 标签 → detected=true + 样本
    #[test]
    fn detect_thinking_finds_tag_and_sample() {
        let resp = r#"data: {"choices":[{"delta":{"content":"<think>推理</think> 答案"}}]}"#;
        let (detected, sample) = detect_thinking(resp);
        assert!(detected);
        assert!(sample.unwrap().contains("推理"));
    }

    /// detect_thinking：无 <think> 标签 → detected=false + None
    #[test]
    fn detect_thinking_negative_for_plain_response() {
        let resp = r#"data: {"choices":[{"delta":{"content":"hello"}}]}"#;
        let (detected, sample) = detect_thinking(resp);
        assert!(!detected);
        assert!(sample.is_none());
    }
}
