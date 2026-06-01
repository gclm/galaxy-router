use crate::api::handlers::admin::channels::EndpointType;

/// 查找 SSE 事件边界（\n\n 或 \r\n\r\n）
pub fn find_sse_boundary(buffer: &[u8]) -> Option<usize> {
    const NN: &[u8] = b"\n\n";
    const RNNN: &[u8] = b"\r\n\r\n";

    if let Some(pos) = buffer.windows(NN.len()).position(|w| w == NN) {
        return Some(pos + NN.len());
    }
    if let Some(pos) = buffer.windows(RNNN.len()).position(|w| w == RNNN) {
        return Some(pos + RNNN.len());
    }
    None
}

/// 提取 SSE 行字段值（兼容 `field: value` 和 `field:value` 两种格式）
#[inline]
pub fn sse_field<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    line.strip_prefix(field)?
        .strip_prefix(':')
        .map(|rest| rest.strip_prefix(' ').unwrap_or(rest))
}

/// SSE 中提取到的 usage 来源
#[derive(Debug, Clone)]
pub enum SseUsageSource {
    /// OpenAI: data 行直接包含 usage
    OpenAi(serde_json::Value),
    /// Anthropic message_start: usage 在 message.usage 中（含 input_tokens）
    AnthropicInput(serde_json::Value),
    /// Anthropic message_delta: usage 在根级（含 output_tokens）
    AnthropicOutput(serde_json::Value),
}

/// 将 SSE usage 提取结果分发到对应变量
#[inline]
pub fn apply_sse_usage(
    source: SseUsageSource,
    last_usage: &mut Option<serde_json::Value>,
    input_usage: &mut Option<serde_json::Value>,
) {
    match source {
        SseUsageSource::OpenAi(v) => *last_usage = Some(v),
        SseUsageSource::AnthropicInput(v) => *input_usage = Some(v),
        SseUsageSource::AnthropicOutput(v) => *last_usage = Some(v),
    }
}

/// 从 SSE 事件中提取 usage 数据（需要完整事件，由 find_sse_boundary 分割）
pub fn extract_usage_from_sse(text: &str, endpoint_type: &EndpointType) -> Option<SseUsageSource> {
    match endpoint_type {
        EndpointType::OpenAiChat | EndpointType::OpenAiResponse => {
            for line in text.lines() {
                if let Some(data) = sse_field(line, "data")
                    && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data)
                    && parsed.get("usage").is_some()
                {
                    return Some(SseUsageSource::OpenAi(parsed));
                }
            }
            None
        }
        EndpointType::Anthropic => {
            let mut event_type = "";
            let mut data = "";
            for line in text.lines() {
                if let Some(stripped) = sse_field(line, "event") {
                    event_type = stripped;
                } else if let Some(stripped) = sse_field(line, "data") {
                    data = stripped;
                }
            }
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) {
                if event_type == "message_start" {
                    if let Some(usage) = parsed.get("message").and_then(|m| m.get("usage")) {
                        return Some(SseUsageSource::AnthropicInput(usage.clone()));
                    }
                    if let Some(usage) = parsed.get("usage") {
                        return Some(SseUsageSource::AnthropicInput(usage.clone()));
                    }
                    tracing::debug!("message_start 无 usage: {}", &data[..data.len().min(500)]);
                } else if event_type == "message_delta" && parsed.get("usage").is_some() {
                    return Some(SseUsageSource::AnthropicOutput(parsed));
                }
            }
            None
        }
        _ => None,
    }
}

/// 从 SSE 事件中提取上游错误。很多供应商会先返回 HTTP 200，再通过 SSE error 事件返回业务错误。
pub fn extract_error_from_sse(text: &str, _endpoint_type: &EndpointType) -> Option<String> {
    let mut event_type = "";
    let mut data_lines = Vec::new();

    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(stripped) = sse_field(line, "event") {
            event_type = stripped.trim();
        } else if let Some(stripped) = sse_field(line, "data") {
            data_lines.push(stripped.trim_start());
        }
    }

    if data_lines.is_empty() {
        return None;
    }

    let data = data_lines.join("\n");
    if data.is_empty() || data == "[DONE]" {
        return None;
    }

    let is_error_event = event_type.eq_ignore_ascii_case("error")
        || event_type.to_ascii_lowercase().contains("error")
        || event_type.eq_ignore_ascii_case("response.failed");

    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&data) {
        if is_error_json(&parsed, is_error_event) {
            return Some(data);
        }
        return None;
    }

    if is_error_event {
        return Some(data);
    }

    None
}

fn is_error_json(value: &serde_json::Value, is_error_event: bool) -> bool {
    if value.get("error").is_some() {
        return true;
    }

    if let Some(t) = value["type"].as_str() {
        let lower = t.to_ascii_lowercase();
        if lower == "error" || lower.contains("error") || lower == "response.failed" {
            return true;
        }
    }

    is_error_event
        && (value.get("message").is_some()
            || value.get("code").is_some()
            || value.get("type").is_some())
}

/// 截断上游错误体，避免泄漏敏感信息
pub fn sanitize_upstream_error(body: &str) -> String {
    let truncated = if body.len() > 500 {
        format!("{}...", &body[..500])
    } else {
        body.to_string()
    };

    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(msg) = v["error"]["message"].as_str() {
            return msg.to_string();
        }
        if let Some(msg) = v["error"].as_str() {
            return msg.to_string();
        }
        if let Some(msg) = v["message"].as_str() {
            return msg.to_string();
        }
    }

    truncated
}

pub fn format_stream_error_event(error_body: &str, client_endpoint: &EndpointType) -> Vec<u8> {
    let message = sanitize_upstream_error(error_body);

    match client_endpoint {
        EndpointType::Anthropic => format!(
            "event: error\ndata: {}\n\n",
            serde_json::json!({
                "type": "error",
                "error": {
                    "type": "api_error",
                    "message": message,
                }
            })
        )
        .into_bytes(),
        EndpointType::OpenAiResponse => format!(
            "event: response.failed\ndata: {}\n\n",
            serde_json::json!({
                "type": "response.failed",
                "error": {
                    "message": message,
                    "type": "server_error",
                }
            })
        )
        .into_bytes(),
        _ => format!(
            "data: {}\n\n",
            serde_json::json!({
                "error": {
                    "message": message,
                    "type": "server_error",
                }
            })
        )
        .into_bytes(),
    }
}

/// 从直通模式的 SSE 文本中提取内容
pub fn collect_sse_content(
    text: &str,
    endpoint_type: &EndpointType,
    output: &mut String,
    reasoning: &mut String,
    tool_calls: &mut Vec<serde_json::Value>,
) {
    match endpoint_type {
        EndpointType::OpenAiChat | EndpointType::OpenAiResponse => {
            for line in text.lines() {
                if let Some(data) = sse_field(line, "data") {
                    if data == "[DONE]" {
                        continue;
                    }
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) {
                        if let Some(content) = parsed["choices"][0]["delta"]["content"].as_str()
                            && !content.is_empty()
                        {
                            output.push_str(content);
                        }
                        if let Some(r) = parsed["choices"][0]["delta"]["reasoning_content"].as_str()
                            && !r.is_empty()
                        {
                            reasoning.push_str(r);
                        }
                        if let Some(tcs) = parsed["choices"][0]["delta"]["tool_calls"].as_array() {
                            for tc in tcs {
                                tool_calls.push(serde_json::json!({
                                    "id": tc["id"],
                                    "name": tc["function"]["name"],
                                    "arguments": tc["function"]["arguments"],
                                }));
                            }
                        }
                    }
                }
            }
        }
        EndpointType::Anthropic => {
            let mut event_type = "";
            let mut data = "";
            for line in text.lines() {
                if let Some(stripped) = sse_field(line, "event") {
                    event_type = stripped.trim();
                } else if let Some(stripped) = sse_field(line, "data") {
                    data = stripped.trim_start();
                }
            }
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data)
                && event_type == "content_block_delta"
            {
                if parsed["delta"]["type"] == "text_delta"
                    && let Some(t) = parsed["delta"]["text"].as_str()
                {
                    output.push_str(t);
                }
                if parsed["delta"]["type"] == "thinking_delta"
                    && let Some(t) = parsed["delta"]["thinking"].as_str()
                {
                    reasoning.push_str(t);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_error_from_sse_detects_openai_error_payload() {
        let event = r#"data: {"error":{"message":"[1113][余额不足或无可用资源包,请充值。]","type":"server_error"}}"#;

        let error = extract_error_from_sse(event, &EndpointType::OpenAiChat).unwrap();

        assert!(error.contains("1113"));
        assert!(error.contains("余额不足"));
    }

    #[test]
    fn extract_error_from_sse_detects_anthropic_error_event() {
        let event = r#"event: error
data: {"type":"error","error":{"type":"api_error","message":"resource exhausted"}}"#;

        let error = extract_error_from_sse(event, &EndpointType::Anthropic).unwrap();

        assert!(error.contains("resource exhausted"));
    }

    #[test]
    fn extract_error_from_sse_detects_responses_failed_event() {
        let event = r#"event: response.failed
data: {"type":"response.failed","response":{"status":"failed"},"error":{"message":"quota exceeded"}}"#;

        let error = extract_error_from_sse(event, &EndpointType::OpenAiResponse).unwrap();

        assert!(error.contains("quota exceeded"));
    }

    #[test]
    fn extract_error_from_sse_ignores_normal_delta() {
        let event = r#"data: {"choices":[{"delta":{"content":"hello"}}]}"#;

        let error = extract_error_from_sse(event, &EndpointType::OpenAiChat);

        assert!(error.is_none());
    }

    #[test]
    fn sanitize_upstream_error_extracts_string_error() {
        let message = sanitize_upstream_error(r#"{"error":"plain upstream error"}"#);

        assert_eq!(message, "plain upstream error");
    }
}
