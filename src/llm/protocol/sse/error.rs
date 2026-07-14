//! SSE 错误提取 + 归因。

use crate::api::handlers::admin::channels::EndpointType;

use super::parsing::sse_field;

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
