//! SSE 事件解析原语 + 直通模式内容累积。

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
