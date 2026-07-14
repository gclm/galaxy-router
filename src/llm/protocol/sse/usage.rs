//! SSE usage 提取（完成检测：usage 随终止事件到达）。

use crate::api::handlers::admin::channels::EndpointType;

use super::parsing::sse_field;

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
        EndpointType::OpenAiChat => {
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
        EndpointType::OpenAiResponse => {
            // OpenAI Responses 流式事件中 usage 在 response.completed 事件的
            // response.usage 路径下，而非顶层
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
                // response.completed 事件：usage 在 parsed["response"]["usage"]
                if event_type == "response.completed"
                    && let Some(usage) = parsed.get("response").and_then(|r| r.get("usage"))
                {
                    return Some(SseUsageSource::OpenAi(usage.clone()));
                }
                // 兼容：usage 也在顶层的情况
                if parsed.get("usage").is_some() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_usage_from_sse_openai_chat_finds_top_level_usage() {
        let event = r#"data: {"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":20}}"#;

        let source = extract_usage_from_sse(event, &EndpointType::OpenAiChat).unwrap();

        match source {
            SseUsageSource::OpenAi(v) => {
                assert_eq!(v["usage"]["prompt_tokens"], 10);
                assert_eq!(v["usage"]["completion_tokens"], 20);
            }
            _ => panic!("expected OpenAi source"),
        }
    }

    #[test]
    fn extract_usage_from_sse_responses_completed_finds_nested_usage() {
        let event = r#"event: response.completed
data: {"type":"response.completed","response":{"id":"resp_123","status":"completed","usage":{"input_tokens":50,"output_tokens":30,"total_tokens":80}}}"#;

        let source = extract_usage_from_sse(event, &EndpointType::OpenAiResponse).unwrap();

        match source {
            SseUsageSource::OpenAi(v) => {
                assert_eq!(v["input_tokens"], 50);
                assert_eq!(v["output_tokens"], 30);
            }
            _ => panic!("expected OpenAi source"),
        }
    }

    #[test]
    fn extract_usage_from_sse_responses_ignores_delta_event() {
        let event = r#"event: response.output_text.delta
data: {"type":"response.output_text.delta","delta":"hello"}"#;

        let source = extract_usage_from_sse(event, &EndpointType::OpenAiResponse);
        assert!(source.is_none());
    }
}
