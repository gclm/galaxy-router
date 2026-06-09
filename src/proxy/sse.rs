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
        EndpointType::OpenAiChat => {
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
                                let idx = tc["index"].as_u64().unwrap_or(0) as usize;
                                let id = tc["id"].as_str().unwrap_or("");
                                let name = tc["function"]["name"].as_str().unwrap_or("");
                                let arguments = tc["function"]["arguments"].as_str().unwrap_or("");

                                // 确保 Vec 足够长
                                while tool_calls.len() <= idx {
                                    tool_calls.push(serde_json::json!({
                                        "id": "",
                                        "name": "",
                                        "arguments": ""
                                    }));
                                }

                                let entry = &mut tool_calls[idx];
                                if !id.is_empty() {
                                    entry["id"] = serde_json::Value::String(id.to_string());
                                }
                                if !name.is_empty() {
                                    entry["name"] = serde_json::Value::String(name.to_string());
                                }
                                if let Some(existing) = entry["arguments"].as_str() {
                                    entry["arguments"] = serde_json::Value::String(format!(
                                        "{}{}",
                                        existing, arguments
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        EndpointType::OpenAiResponse => {
            // OpenAI Responses 流式事件使用不同的格式：
            // event: response.output_text.delta  → { "delta": "text", ... }
            // event: response.reasoning.delta    → { "delta": "text", ... }
            // event: response.function_call_arguments.delta → { "delta": "args", "call_id", "name" }
            let mut event_type = "";
            let mut data = "";
            for line in text.lines() {
                if let Some(stripped) = sse_field(line, "event") {
                    event_type = stripped.trim();
                } else if let Some(stripped) = sse_field(line, "data") {
                    data = stripped.trim_start();
                }
            }
            if data.is_empty() || data == "[DONE]" {
                return;
            }
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) {
                match event_type {
                    "response.output_text.delta" => {
                        if let Some(delta) = parsed["delta"].as_str()
                            && !delta.is_empty()
                        {
                            output.push_str(delta);
                        }
                    }
                    "response.reasoning.delta" => {
                        if let Some(delta) = parsed["delta"].as_str()
                            && !delta.is_empty()
                        {
                            reasoning.push_str(delta);
                        }
                    }
                    "response.function_call_arguments.delta" => {
                        let call_id = parsed["call_id"].as_str().unwrap_or("");
                        let name = parsed["name"].as_str().unwrap_or("");
                        let delta = parsed["delta"].as_str().unwrap_or("");

                        if let Some(existing) = tool_calls
                            .iter_mut()
                            .find(|tc| tc["id"].as_str() == Some(call_id))
                        {
                            if !name.is_empty() {
                                existing["name"] = serde_json::Value::String(name.to_string());
                            }
                            if let Some(args) = existing["arguments"].as_str() {
                                existing["arguments"] =
                                    serde_json::Value::String(format!("{}{}", args, delta));
                            }
                        } else {
                            tool_calls.push(serde_json::json!({
                                "id": call_id,
                                "name": name,
                                "arguments": delta,
                            }));
                        }
                    }
                    _ => {}
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

    #[test]
    fn collect_sse_content_responses_text_delta() {
        let event = r#"event: response.output_text.delta
data: {"type":"response.output_text.delta","delta":"Hello world","output_index":0,"content_index":0}"#;

        let mut output = String::new();
        let mut reasoning = String::new();
        let mut tool_calls = Vec::new();
        collect_sse_content(
            event,
            &EndpointType::OpenAiResponse,
            &mut output,
            &mut reasoning,
            &mut tool_calls,
        );

        assert_eq!(output, "Hello world");
        assert!(reasoning.is_empty());
        assert!(tool_calls.is_empty());
    }

    #[test]
    fn collect_sse_content_responses_reasoning_delta() {
        let event = r#"event: response.reasoning.delta
data: {"type":"response.reasoning.delta","delta":"thinking...","output_index":0}"#;

        let mut output = String::new();
        let mut reasoning = String::new();
        let mut tool_calls = Vec::new();
        collect_sse_content(
            event,
            &EndpointType::OpenAiResponse,
            &mut output,
            &mut reasoning,
            &mut tool_calls,
        );

        assert!(output.is_empty());
        assert_eq!(reasoning, "thinking...");
        assert!(tool_calls.is_empty());
    }

    #[test]
    fn collect_sse_content_responses_tool_call_delta() {
        let event = r#"event: response.function_call_arguments.delta
data: {"type":"response.function_call_arguments.delta","call_id":"call_123","name":"get_weather","delta":"{\"city\":"}"#;

        let mut output = String::new();
        let mut reasoning = String::new();
        let mut tool_calls = Vec::new();
        collect_sse_content(
            event,
            &EndpointType::OpenAiResponse,
            &mut output,
            &mut reasoning,
            &mut tool_calls,
        );

        assert!(output.is_empty());
        assert!(reasoning.is_empty());
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0]["name"], "get_weather");
        assert_eq!(tool_calls[0]["id"], "call_123");
        assert_eq!(tool_calls[0]["arguments"], "{\"city\":");
    }

    #[test]
    fn collect_sse_content_chat_tool_calls_accumulates_multi_chunk() {
        // 模拟 OpenAI Chat 流式 tool_calls 分片：
        // chunk1: id + name + 首段 arguments
        // chunk2: 续传 arguments
        // chunk3: 续传 arguments
        let chunk1 = r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_abc","type":"function","function":{"name":"get_weather","arguments":"{\"ci"}}]}}]}"#;
        let chunk2 = r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"ty\": \"N"}}]}}]}"#;
        let chunk3 = r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"ew York\"}"}}]}}]}"#;

        let mut output = String::new();
        let mut reasoning = String::new();
        let mut tool_calls = Vec::new();

        for chunk in &[chunk1, chunk2, chunk3] {
            collect_sse_content(
                chunk,
                &EndpointType::OpenAiChat,
                &mut output,
                &mut reasoning,
                &mut tool_calls,
            );
        }

        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0]["id"], "call_abc");
        assert_eq!(tool_calls[0]["name"], "get_weather");
        assert_eq!(tool_calls[0]["arguments"], "{\"city\": \"New York\"}");
    }

    #[test]
    fn collect_sse_content_chat_tool_calls_parallel() {
        // 并行 tool calls：两个不同的函数调用交错到达
        let chunk1 = r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"fn_a","arguments":"{\"a\":"}}]}}]}"#;
        let chunk2 = r#"data: {"choices":[{"delta":{"tool_calls":[{"index":1,"id":"call_b","type":"function","function":{"name":"fn_b","arguments":"{\"b\":"}}]}}]}"#;
        let chunk3 = r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"1}"}}]}}]}"#;
        let chunk4 = r#"data: {"choices":[{"delta":{"tool_calls":[{"index":1,"function":{"arguments":"2}"}}]}}]}"#;

        let mut output = String::new();
        let mut reasoning = String::new();
        let mut tool_calls = Vec::new();

        for chunk in &[chunk1, chunk2, chunk3, chunk4] {
            collect_sse_content(
                chunk,
                &EndpointType::OpenAiChat,
                &mut output,
                &mut reasoning,
                &mut tool_calls,
            );
        }

        assert_eq!(tool_calls.len(), 2);
        assert_eq!(tool_calls[0]["id"], "call_a");
        assert_eq!(tool_calls[0]["name"], "fn_a");
        assert_eq!(tool_calls[0]["arguments"], "{\"a\":1}");
        assert_eq!(tool_calls[1]["id"], "call_b");
        assert_eq!(tool_calls[1]["name"], "fn_b");
        assert_eq!(tool_calls[1]["arguments"], "{\"b\":2}");
    }

    #[test]
    fn collect_sse_content_responses_tool_calls_accumulates_multi_chunk() {
        let chunk1 = r#"event: response.function_call_arguments.delta
data: {"type":"response.function_call_arguments.delta","call_id":"call_123","name":"get_weather","delta":"{\"ci"}"#;
        let chunk2 = r#"event: response.function_call_arguments.delta
data: {"type":"response.function_call_arguments.delta","call_id":"call_123","delta":"ty\": \"NY\"}"}"#;

        let mut output = String::new();
        let mut reasoning = String::new();
        let mut tool_calls = Vec::new();

        for chunk in &[chunk1, chunk2] {
            collect_sse_content(
                chunk,
                &EndpointType::OpenAiResponse,
                &mut output,
                &mut reasoning,
                &mut tool_calls,
            );
        }

        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0]["id"], "call_123");
        assert_eq!(tool_calls[0]["name"], "get_weather");
        assert_eq!(tool_calls[0]["arguments"], "{\"city\": \"NY\"}");
    }
}
