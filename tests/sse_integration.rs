//! SSE 流式响应解析与转换集成测试
//!
//! 覆盖 proxy/sse 模块的纯函数路径。

use galaxy_router::domain::channel::EndpointType;
use galaxy_router::llm::protocol::sse::{
    SseUsageSource, apply_sse_usage, collect_sse_content, extract_error_from_sse,
    extract_usage_from_sse, find_sse_boundary, format_stream_error_event, sanitize_upstream_error,
    sse_field,
};

#[test]
fn test_find_sse_boundary_returns_double_newline_offset() {
    let buffer = b"data: foo\n\nmore";
    let boundary = find_sse_boundary(buffer).unwrap();
    // boundary 是第二个 \n 之后的位置（含 \n\n）
    assert_eq!(&buffer[..boundary], b"data: foo\n\n");
}

#[test]
fn test_find_sse_boundary_returns_none_when_no_double_newline() {
    let buffer = b"data: foo\ndata: bar";
    assert!(find_sse_boundary(buffer).is_none());
}

#[test]
fn test_find_sse_boundary_handles_crlf() {
    let buffer = b"data: foo\r\n\r\nmore";
    let boundary = find_sse_boundary(buffer).unwrap();
    assert!(boundary < buffer.len());
    assert_eq!(&buffer[..boundary], b"data: foo\r\n\r\n");
}

#[test]
fn test_sse_field_extracts_data_value() {
    assert_eq!(sse_field("data: hello", "data"), Some("hello"));
    assert_eq!(sse_field("data: hello world", "data"), Some("hello world"));
    assert_eq!(sse_field("event: foo", "data"), None);
}

#[test]
fn test_sse_field_handles_prefix_whitespace() {
    // SSE 规范：字段名前的单个空格可忽略
    assert_eq!(sse_field("data:hello", "data"), Some("hello"));
}

#[test]
fn test_sanitize_upstream_error_truncates_long_bodies() {
    let long = "x".repeat(2000);
    let result = sanitize_upstream_error(&long);
    assert!(result.len() <= 1024);
    assert!(result.contains("..."));
}

#[test]
fn test_sanitize_upstream_error_keeps_short_bodies_intact() {
    let body = r#"{"error": "rate limited"}"#;
    let result = sanitize_upstream_error(body);
    assert!(result.contains("rate limited"));
}

#[test]
fn test_format_stream_error_event_openai_chat_uses_data_only() {
    let bytes = format_stream_error_event(r#"{"error": "invalid key"}"#, &EndpointType::OpenAiChat);
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.starts_with("data: "));
    assert!(text.ends_with("\n\n"));
    assert!(text.contains("invalid key"));
    assert!(!text.contains("event:"));
}

#[test]
fn test_format_stream_error_event_anthropic_uses_event_and_data() {
    let bytes = format_stream_error_event(r#"{"error": "overloaded"}"#, &EndpointType::Anthropic);
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("event: error"));
    assert!(text.contains("data: "));
    assert!(text.contains("overloaded"));
}

#[test]
fn test_collect_sse_content_openai_chat_concatenates_deltas() {
    let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"hello \"}}]}\n\
               data: {\"choices\":[{\"delta\":{\"content\":\"world\"}}]}\n\
               data: [DONE]\n";
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut tool_calls = Vec::new();
    collect_sse_content(
        sse,
        &EndpointType::OpenAiChat,
        &mut content,
        &mut reasoning,
        &mut tool_calls,
    );
    assert_eq!(content, "hello world");
    assert!(reasoning.is_empty());
    assert!(tool_calls.is_empty());
}

#[test]
fn test_collect_sse_content_openai_chat_collects_reasoning() {
    let sse = "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"thinking...\"}}]}\n\
               data: {\"choices\":[{\"delta\":{\"content\":\"answer\"}}]}\n\
               data: [DONE]\n";
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut tool_calls = Vec::new();
    collect_sse_content(
        sse,
        &EndpointType::OpenAiChat,
        &mut content,
        &mut reasoning,
        &mut tool_calls,
    );
    assert_eq!(content, "answer");
    assert_eq!(reasoning, "thinking...");
}

#[test]
fn test_collect_sse_content_anthropic_uses_delta_text_field() {
    // Anthropic 解析单 event 块（当前实现：只处理最后遇到的事件）
    let sse = "event: content_block_delta\n\
               data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi there\"}}\n\n";
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut tool_calls = Vec::new();
    collect_sse_content(
        sse,
        &EndpointType::Anthropic,
        &mut content,
        &mut reasoning,
        &mut tool_calls,
    );
    assert_eq!(content, "Hi there");
}

#[test]
fn test_extract_usage_from_sse_openai_chat() {
    let sse = "data: {\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":20}}\n";
    let usage = extract_usage_from_sse(sse, &EndpointType::OpenAiChat).unwrap();
    let SseUsageSource::OpenAi(v) = usage else {
        panic!("expected OpenAi variant");
    };
    // 返回值是完整 data 行（外层包了 usage），所以从 usage 子对象读
    assert_eq!(v["usage"]["prompt_tokens"], 10);
    assert_eq!(v["usage"]["completion_tokens"], 20);
}

#[test]
fn test_extract_usage_from_sse_anthropic_message_start() {
    // Anthropic message_start 事件：usage 在 message.usage 下
    let sse = "event: message_start\n\
               data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":7}}}\n";
    let usage = extract_usage_from_sse(sse, &EndpointType::Anthropic).unwrap();
    let SseUsageSource::AnthropicInput(v) = usage else {
        panic!("expected AnthropicInput variant");
    };
    assert_eq!(v["input_tokens"], 7);
}

#[test]
fn test_extract_usage_from_sse_anthropic_message_delta() {
    // Anthropic message_delta 事件：usage 在根级
    let sse = "event: message_delta\n\
               data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":13}}\n";
    let usage = extract_usage_from_sse(sse, &EndpointType::Anthropic).unwrap();
    let SseUsageSource::AnthropicOutput(v) = usage else {
        panic!("expected AnthropicOutput variant");
    };
    // 返回值是 data 根对象，从 usage 子对象读
    assert_eq!(v["usage"]["output_tokens"], 13);
}

#[test]
fn test_extract_usage_from_sse_returns_none_when_no_usage() {
    let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n\
               data: [DONE]\n";
    assert!(extract_usage_from_sse(sse, &EndpointType::OpenAiChat).is_none());
}

#[test]
fn test_extract_error_from_sse_openai_chat() {
    // 单个 data 行的 OpenAI 错误载荷
    let sse = "data: {\"error\":{\"message\":\"quota exceeded\"}}\n\n";
    let err = extract_error_from_sse(sse, &EndpointType::OpenAiChat).unwrap();
    assert!(err.contains("quota exceeded"));
}

#[test]
fn test_extract_error_from_sse_returns_none_on_success() {
    let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\
               data: [DONE]\n";
    assert!(extract_error_from_sse(sse, &EndpointType::OpenAiChat).is_none());
}

#[test]
fn test_apply_sse_usage_stores_in_correct_slot() {
    let source = SseUsageSource::OpenAi(serde_json::json!({"prompt_tokens": 5}));
    let mut last_usage = None;
    let mut input_usage = None;
    apply_sse_usage(source, &mut last_usage, &mut input_usage);
    assert!(last_usage.is_some());
    assert!(input_usage.is_none());

    // Anthropic input 写到 input_usage
    let source = SseUsageSource::AnthropicInput(serde_json::json!({"input_tokens": 3}));
    let mut last_usage = None;
    let mut input_usage = None;
    apply_sse_usage(source, &mut last_usage, &mut input_usage);
    assert!(last_usage.is_none());
    assert_eq!(input_usage.unwrap()["input_tokens"], 3);
}
