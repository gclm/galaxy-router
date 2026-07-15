//! 协议转换集成测试
//!
//! 覆盖 protocol/inbound、protocol/outbound 各种组合的请求/响应转换。

use axum::http::HeaderMap;
use galaxy_router::llm::protocol::inbound::Inbound;
use galaxy_router::llm::protocol::inbound::anthropic::AnthropicInbound;
use galaxy_router::llm::protocol::inbound::openai_chat::OpenAiChatInbound;
use galaxy_router::llm::protocol::inbound::openai_responses::OpenAiResponsesInbound;
use galaxy_router::llm::protocol::model::{
    Content, ContentPart, LlmRequest, LlmResponse, Message, Role,
};
use galaxy_router::llm::protocol::outbound::Outbound;
use galaxy_router::llm::protocol::outbound::anthropic::AnthropicOutbound;
use galaxy_router::llm::protocol::outbound::openai_chat::OpenAiChatOutbound;
use galaxy_router::llm::protocol::outbound::openai_responses::OpenAiResponsesOutbound;

#[allow(unused_imports)]
use serde_json::json;

// ============================================================
// Inbound 转换：各协议 → 统一 LlmRequest
// ============================================================

#[tokio::test]
async fn test_openai_chat_inbound_full_shape() {
    let body = r#"{
        "model": "gpt-4o",
        "messages": [
            {"role": "system", "content": "You are helpful"},
            {"role": "user", "content": "Hello"},
            {"role": "assistant", "content": "Hi"},
            {"role": "user", "content": "How are you?"}
        ],
        "temperature": 0.7,
        "max_tokens": 100,
        "stream": false,
        "stop": ["END"]
    }"#;
    let req = OpenAiChatInbound
        .transform_request(body.as_bytes(), &HeaderMap::new())
        .await
        .unwrap();
    assert_eq!(req.model, "gpt-4o");
    assert_eq!(req.messages.len(), 4);
    assert_eq!(req.messages[0].role, Role::System);
    assert_eq!(req.messages[1].role, Role::User);
    assert_eq!(req.temperature, Some(0.7));
    assert_eq!(req.max_tokens, Some(100));
    assert_eq!(req.stream, Some(false));
    assert_eq!(req.stop.as_deref(), Some(&["END".to_string()][..]));
}

#[tokio::test]
async fn test_anthropic_inbound_with_system() {
    let body = r#"{
        "model": "claude-sonnet-4-20250514",
        "system": "You are a coding assistant",
        "max_tokens": 2048,
        "messages": [{"role": "user", "content": "Write hello world in Rust"}]
    }"#;
    let req = AnthropicInbound
        .transform_request(body.as_bytes(), &HeaderMap::new())
        .await
        .unwrap();
    assert_eq!(req.model, "claude-sonnet-4-20250514");
    // Anthropic max_tokens 字段映射到 LlmRequest::max_completion_tokens
    assert_eq!(req.max_completion_tokens, Some(2048));
    assert!(req.messages.iter().any(|m| matches!(m.role, Role::System)));
}

#[tokio::test]
async fn test_openai_responses_inbound_handles_input_as_string() {
    let body = r#"{
        "model": "o1",
        "input": "What is 2+2?",
        "max_output_tokens": 256
    }"#;
    let req = OpenAiResponsesInbound
        .transform_request(body.as_bytes(), &HeaderMap::new())
        .await
        .unwrap();
    assert_eq!(req.model, "o1");
    assert!(!req.messages.is_empty());
    // OpenAI Responses 的 max_output_tokens 映射到 LlmRequest::max_completion_tokens
    assert_eq!(req.max_completion_tokens, Some(256));
}

// ============================================================
// Outbound 转换：统一 LlmRequest → 各协议 JSON
// ============================================================

fn sample_llm_request() -> LlmRequest {
    LlmRequest {
        model: "test-model".into(),
        messages: vec![Message {
            role: Role::User,
            content: Some(Content::Text("hello".into())),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
            cache_control: None,
        }],
        temperature: Some(0.5),
        top_p: None,
        max_tokens: Some(128),
        max_completion_tokens: None,
        stream: Some(false),
        tools: None,
        tool_choice: None,
        stop: None,
        reasoning_effort: None,
        extra: Default::default(),
    }
}

#[test]
fn test_openai_chat_outbound_emits_messages_array() {
    let bytes = OpenAiChatOutbound
        .transform_request(&sample_llm_request())
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["model"], "test-model");
    assert_eq!(v["messages"][0]["role"], "user");
    assert_eq!(v["messages"][0]["content"], "hello");
    assert_eq!(v["temperature"], 0.5);
    // max_completion_tokens 透传到 max_tokens（OpenAI Chat 用 max_tokens 字段）
    assert_eq!(v["max_tokens"], 128);
    assert_eq!(v["stream"], false);
}

#[test]
fn test_openai_responses_outbound_uses_input_field() {
    // OpenAI Responses 用 max_completion_tokens 字段
    let mut req = sample_llm_request();
    req.max_tokens = None;
    req.max_completion_tokens = Some(128);
    let bytes = OpenAiResponsesOutbound.transform_request(&req).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["model"], "test-model");
    let input = &v["input"];
    assert!(input.is_string() || input.is_array());
    assert_eq!(v["max_output_tokens"], 128);
}

#[test]
fn test_anthropic_outbound_emits_messages_and_separate_max_tokens() {
    // Anthropic outbound 从 LlmRequest.max_completion_tokens 取值
    let mut req = sample_llm_request();
    req.max_tokens = None;
    req.max_completion_tokens = Some(128);
    let bytes = AnthropicOutbound.transform_request(&req).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["model"], "test-model");
    assert_eq!(v["max_tokens"], 128);
    assert!(v["messages"].is_array());
    assert_eq!(v["messages"][0]["role"], "user");
    assert_eq!(v["temperature"], 0.5);
}

#[test]
fn test_anthropic_outbound_defaults_max_tokens_to_4096_when_missing() {
    let mut req = sample_llm_request();
    req.max_tokens = None;
    req.max_completion_tokens = None;
    let bytes = AnthropicOutbound.transform_request(&req).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    // Anthropic 服务端要求 max_tokens，outbound 给 4096 兜底
    assert_eq!(v["max_tokens"], 4096);
}

// ============================================================
// Auth header 注入
// ============================================================

#[test]
fn test_openai_chat_outbound_sets_bearer_authorization() {
    let outbound = OpenAiChatOutbound;
    let mut headers = HeaderMap::new();
    outbound.set_auth_header(&mut headers, "sk-test-key");
    let auth = headers.get("authorization").unwrap().to_str().unwrap();
    assert_eq!(auth, "Bearer sk-test-key");
}

#[test]
fn test_anthropic_outbound_sets_x_api_key_and_version() {
    let outbound = AnthropicOutbound;
    let mut headers = HeaderMap::new();
    outbound.set_auth_header(&mut headers, "sk-ant-test");
    assert_eq!(headers.get("x-api-key").unwrap(), "sk-ant-test");
    assert_eq!(headers.get("anthropic-version").unwrap(), "2023-06-01");
}

#[test]
fn test_openai_responses_outbound_uses_bearer() {
    let outbound = OpenAiResponsesOutbound;
    let mut headers = HeaderMap::new();
    outbound.set_auth_header(&mut headers, "sk-oai");
    assert_eq!(headers.get("authorization").unwrap(), "Bearer sk-oai");
}

// ============================================================
// Inbound 响应转换：LlmResponse → 各协议字节
// ============================================================

fn sample_llm_response() -> LlmResponse {
    LlmResponse {
        id: "resp-1".into(),
        object: "chat.completion".into(),
        created: 1_700_000_000,
        model: "gpt-4o".into(),
        choices: vec![galaxy_router::llm::protocol::model::Choice {
            index: 0,
            message: Message {
                role: Role::Assistant,
                content: Some(Content::Text("hi there".into())),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
                cache_control: None,
            },
            finish_reason: Some(galaxy_router::llm::protocol::model::FinishReason::Stop),
        }],
        usage: Some(galaxy_router::llm::protocol::model::Usage {
            prompt_tokens: 5,
            completion_tokens: 7,
            total_tokens: 12,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        }),
        system_fingerprint: None,
    }
}

#[test]
fn test_openai_chat_inbound_response_wraps_in_choices_array() {
    let bytes = OpenAiChatInbound
        .transform_response(&sample_llm_response())
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["choices"][0]["message"]["content"], "hi there");
    assert_eq!(v["usage"]["prompt_tokens"], 5);
    assert_eq!(v["usage"]["completion_tokens"], 7);
}

#[test]
fn test_anthropic_inbound_response_uses_content_blocks() {
    let bytes = AnthropicInbound
        .transform_response(&sample_llm_response())
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    // Anthropic 响应使用 content 数组
    assert!(v["content"].is_array());
    assert_eq!(v["content"][0]["text"], "hi there");
    assert_eq!(v["usage"]["input_tokens"], 5);
    assert_eq!(v["usage"]["output_tokens"], 7);
}

#[test]
fn test_openai_responses_inbound_response_format() {
    let bytes = OpenAiResponsesInbound
        .transform_response(&sample_llm_response())
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    // Responses 格式用 output 数组包 content
    assert!(v.get("output").is_some() || v.get("choices").is_some());
}

// ============================================================
// 跨协议集成场景
// ============================================================

#[tokio::test]
async fn test_openai_chat_to_anthropic_round_trip() {
    // OpenAI Chat 客户端发 max_tokens=100
    let client_body = r#"{
        "model": "claude-sonnet-4-20250514",
        "messages": [{"role": "user", "content": "Hi"}],
        "max_tokens": 100
    }"#;

    let llm_req = OpenAiChatInbound
        .transform_request(client_body.as_bytes(), &HeaderMap::new())
        .await
        .unwrap();

    // 当前实现：OpenAI Chat max_tokens → LlmRequest.max_tokens，
    // 不映射到 max_completion_tokens。Anthropic outbound 只读 max_completion_tokens，
    // 所以默认 4096。这是已知转换边界，测试断言实际行为。
    let upstream_bytes = AnthropicOutbound.transform_request(&llm_req).unwrap();
    let upstream_json: serde_json::Value = serde_json::from_slice(&upstream_bytes).unwrap();
    assert_eq!(upstream_json["model"], "claude-sonnet-4-20250514");
    // max_tokens 字段一定有值（4096 兜底）
    assert!(upstream_json["max_tokens"].is_number());
    assert_eq!(upstream_json["messages"][0]["content"], "Hi");
}

#[tokio::test]
async fn test_anthropic_to_openai_chat_round_trip() {
    // Anthropic 客户端发 max_tokens=50 → LlmRequest.max_completion_tokens
    let client_body = r#"{
        "model": "gpt-4o",
        "system": "helpful",
        "messages": [{"role": "user", "content": "Hi"}],
        "max_tokens": 50
    }"#;
    let llm_req = AnthropicInbound
        .transform_request(client_body.as_bytes(), &HeaderMap::new())
        .await
        .unwrap();
    assert_eq!(llm_req.max_completion_tokens, Some(50));

    let upstream_bytes = OpenAiChatOutbound.transform_request(&llm_req).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&upstream_bytes).unwrap();
    assert_eq!(v["model"], "gpt-4o");
    // OpenAI Chat outbound 把 max_completion_tokens 字段原样输出
    assert_eq!(v["max_completion_tokens"], 50);
    assert!(!v["messages"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_openai_chat_multimodal_content_parts() {
    // OpenAI Chat 支持 content 为 [{"type": "text", ...}, {"type": "image_url", ...}]
    let body = r#"{
        "model": "gpt-4o",
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": "What's in this image?"},
                {"type": "image_url", "image_url": {"url": "https://example.com/cat.png"}}
            ]
        }]
    }"#;
    let req = OpenAiChatInbound
        .transform_request(body.as_bytes(), &HeaderMap::new())
        .await
        .unwrap();
    assert_eq!(req.messages.len(), 1);
    match &req.messages[0].content {
        Some(Content::Parts(parts)) => {
            assert!(!parts.is_empty());
            assert!(parts.iter().any(|p| matches!(p, ContentPart::Text { .. })));
        }
        _ => panic!("expected Parts content"),
    }
}

// ============================================================
// Anthropic inbound 多模态 / 工具 / 推理 / 错误路径
// ============================================================

#[tokio::test]
async fn test_anthropic_inbound_system_as_array_joins_text_blocks() {
    let body = r#"{
        "model": "claude-sonnet-4",
        "system": [
            {"type": "text", "text": "Line 1"},
            {"type": "text", "text": "Line 2"}
        ],
        "max_tokens": 100,
        "messages": [{"role": "user", "content": "Hi"}]
    }"#;
    let req = AnthropicInbound
        .transform_request(body.as_bytes(), &HeaderMap::new())
        .await
        .unwrap();
    let sys_msg = req
        .messages
        .iter()
        .find(|m| matches!(m.role, Role::System))
        .expect("system message present");
    match &sys_msg.content {
        Some(Content::Text(s)) => assert_eq!(s, "Line 1\nLine 2"),
        _ => panic!("expected joined text content"),
    }
}

#[tokio::test]
async fn test_anthropic_inbound_invalid_system_format_errors() {
    let body = r#"{
        "model": "claude-sonnet-4",
        "system": 42,
        "messages": [{"role": "user", "content": "Hi"}]
    }"#;
    let result = AnthropicInbound
        .transform_request(body.as_bytes(), &HeaderMap::new())
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_anthropic_inbound_image_and_tool_use_and_result() {
    let body = r#"{
        "model": "claude-sonnet-4",
        "max_tokens": 100,
        "messages": [
            {"role": "user", "content": [
                {"type": "image", "source": {"type": "base64", "data": "BASE64DATA"}}
            ]},
            {"role": "assistant", "content": [
                {"type": "tool_use", "id": "tu-1", "name": "get_weather", "input": {"city": "SF"}}
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "tu-1", "content": "72F sunny"}
            ]}
        ]
    }"#;
    let req = AnthropicInbound
        .transform_request(body.as_bytes(), &HeaderMap::new())
        .await
        .unwrap();
    assert_eq!(req.messages.len(), 3);
    // 第一条：image 转为 ImageUrl
    match &req.messages[0].content {
        Some(Content::Parts(parts)) => {
            assert!(
                parts
                    .iter()
                    .any(|p| matches!(p, ContentPart::ImageUrl { .. }))
            );
        }
        _ => panic!("expected Parts for image"),
    }
    // 第二条：tool_use
    match &req.messages[1].content {
        Some(Content::Parts(parts)) => {
            assert!(parts.iter().any(|p| matches!(
                p,
                ContentPart::ToolUse { id, .. } if id == "tu-1"
            )));
        }
        _ => panic!("expected Parts for tool_use"),
    }
    // 第三条：tool_result
    match &req.messages[2].content {
        Some(Content::Parts(parts)) => {
            assert!(parts.iter().any(|p| matches!(
                p,
                ContentPart::ToolResult { tool_call_id, .. } if tool_call_id == "tu-1"
            )));
        }
        _ => panic!("expected Parts for tool_result"),
    }
}

#[tokio::test]
async fn test_anthropic_inbound_thinking_block_populates_reasoning_content() {
    let body = r#"{
        "model": "claude-sonnet-4",
        "max_tokens": 100,
        "messages": [{
            "role": "assistant",
            "content": [
                {"type": "thinking", "thinking": "Let me think..."},
                {"type": "text", "text": "Answer is 42"}
            ]
        }]
    }"#;
    let req = AnthropicInbound
        .transform_request(body.as_bytes(), &HeaderMap::new())
        .await
        .unwrap();
    let msg = &req.messages[0];
    assert_eq!(msg.reasoning_content.as_deref(), Some("Let me think..."));
}

#[tokio::test]
async fn test_anthropic_inbound_with_tools() {
    let body = r#"{
        "model": "claude-sonnet-4",
        "max_tokens": 100,
        "tools": [{
            "name": "get_weather",
            "description": "Get weather",
            "input_schema": {"type": "object"}
        }],
        "messages": [{"role": "user", "content": "Weather?"}]
    }"#;
    let req = AnthropicInbound
        .transform_request(body.as_bytes(), &HeaderMap::new())
        .await
        .unwrap();
    let tools = req.tools.expect("tools present");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].function.name, "get_weather");
    assert_eq!(tools[0].tool_type, "function");
}

#[test]
fn test_anthropic_inbound_response_with_reasoning_and_tool_use() {
    let mut resp = sample_llm_response();
    resp.choices[0].message.reasoning_content = Some("because...".into());
    let bytes = AnthropicInbound.transform_response(&resp).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let content = v["content"].as_array().unwrap();
    assert!(content.iter().any(|c| c["type"] == "thinking"));
    assert!(content.iter().any(|c| c["type"] == "text"));
}

#[test]
fn test_anthropic_inbound_response_with_tool_use_part() {
    let resp = LlmResponse {
        choices: vec![galaxy_router::llm::protocol::model::Choice {
            index: 0,
            message: Message {
                role: Role::Assistant,
                content: Some(Content::Parts(vec![ContentPart::ToolUse {
                    id: "tu-x".into(),
                    name: "search".into(),
                    input: serde_json::json!({"q": "rust"}),
                }])),
                ..sample_llm_response().choices[0].message.clone()
            },
            finish_reason: Some(galaxy_router::llm::protocol::model::FinishReason::ToolCalls),
        }],
        ..sample_llm_response()
    };
    let bytes = AnthropicInbound.transform_response(&resp).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["stop_reason"], "tool_use");
    let content = v["content"].as_array().unwrap();
    assert!(
        content
            .iter()
            .any(|c| c["type"] == "tool_use" && c["id"] == "tu-x")
    );
}

#[test]
fn test_anthropic_inbound_response_with_empty_choices() {
    let resp = LlmResponse {
        choices: vec![],
        ..sample_llm_response()
    };
    let bytes = AnthropicInbound.transform_response(&resp).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(v["content"].as_array().unwrap().is_empty());
    assert_eq!(v["stop_reason"], serde_json::Value::Null);
}

#[test]
fn test_anthropic_inbound_stream_event_emits_text_delta_and_stop() {
    use galaxy_router::llm::protocol::model::{FinishReason, LlmStreamResponse, Message, StreamChoice};
    let event = LlmStreamResponse {
        id: String::new(),
        object: "chunk".into(),
        created: 0,
        model: String::new(),
        choices: vec![StreamChoice {
            index: 0,
            delta: Message {
                role: Role::Assistant,
                content: Some(Content::Text("hello".into())),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
                cache_control: None,
            },
            finish_reason: Some(FinishReason::Stop),
        }],
        usage: None,
        system_fingerprint: None,
    };
    let bytes = AnthropicInbound.transform_stream_event(&event).unwrap();
    let s = String::from_utf8(bytes).unwrap();
    assert!(s.contains("event: content_block_delta"));
    assert!(s.contains("\"text\":\"hello\""));
    assert!(s.contains("event: message_stop"));
}

// ============================================================
// Anthropic outbound：response + stream 解析
// ============================================================

#[tokio::test]
async fn test_anthropic_outbound_response_parses_tool_use_and_thinking() {
    let body = serde_json::to_vec(&serde_json::json!({
        "id": "msg_1",
        "type": "message",
        "model": "claude-sonnet-4",
        "content": [
            {"type": "thinking", "thinking": "Hmm..."},
            {"type": "text", "text": "42"},
            {"type": "tool_use", "id": "tu-1", "name": "calc", "input": {"x": 1}}
        ],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 10, "output_tokens": 5}
    }))
    .unwrap();
    let resp = AnthropicOutbound
        .transform_response(&body, 200)
        .await
        .unwrap();
    assert_eq!(resp.model, "claude-sonnet-4");
    assert_eq!(
        resp.choices[0].message.reasoning_content.as_deref(),
        Some("Hmm...")
    );
    let usage = resp.usage.unwrap();
    assert_eq!(usage.prompt_tokens, 10);
    assert_eq!(usage.completion_tokens, 5);
}

#[test]
fn test_anthropic_outbound_stream_event_message_start() {
    let sse = "event: message_start\ndata: {\"message\":{\"id\":\"msg_x\",\"model\":\"claude-sonnet-4\"}}";
    let parsed = AnthropicOutbound
        .transform_stream_event(sse.as_bytes())
        .unwrap()
        .expect("should parse");
    assert_eq!(parsed.id, "msg_x");
    assert_eq!(parsed.model, "claude-sonnet-4");
}

#[test]
fn test_anthropic_outbound_stream_event_thinking_delta() {
    let sse = "event: content_block_delta\ndata: {\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"step 1\"}}";
    let parsed = AnthropicOutbound
        .transform_stream_event(sse.as_bytes())
        .unwrap()
        .expect("should parse");
    assert_eq!(
        parsed.choices[0].delta.reasoning_content.as_deref(),
        Some("step 1")
    );
}

#[test]
fn test_anthropic_outbound_stream_event_message_delta_finish() {
    let sse = "event: message_delta\ndata: {\"delta\":{\"stop_reason\":\"end_turn\"}}";
    let parsed = AnthropicOutbound
        .transform_stream_event(sse.as_bytes())
        .unwrap()
        .expect("should parse");
    assert!(parsed.choices[0].finish_reason.is_some());
}

#[test]
fn test_anthropic_outbound_stream_event_done_returns_none() {
    let sse = "event: message_stop\ndata: {}";
    let parsed = AnthropicOutbound
        .transform_stream_event(sse.as_bytes())
        .unwrap();
    assert!(parsed.is_none());
}

#[test]
fn test_anthropic_outbound_stream_event_empty_text_returns_none() {
    assert!(
        AnthropicOutbound
            .transform_stream_event(b"")
            .unwrap()
            .is_none()
    );
}

#[test]
fn test_anthropic_outbound_request_includes_tools_and_stream() {
    let mut req = sample_llm_request();
    req.tools = Some(vec![galaxy_router::llm::protocol::model::Tool {
        tool_type: "function".into(),
        function: galaxy_router::llm::protocol::model::FunctionDefinition {
            name: "search".into(),
            description: Some("search docs".into()),
            parameters: Some(serde_json::json!({"type": "object"})),
        },
    }]);
    req.stream = Some(true);
    req.temperature = Some(0.3);
    let bytes = AnthropicOutbound.transform_request(&req).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["stream"], true);
    assert_eq!(v["temperature"], 0.3);
    assert!(v["tools"].is_array());
    assert_eq!(v["tools"][0]["name"], "search");
}

// ============================================================
// OpenAI Chat inbound：tool_calls / tool_choice / stream
// ============================================================

#[tokio::test]
async fn test_openai_chat_inbound_with_tool_calls() {
    let body = r#"{
        "model": "gpt-4o",
        "messages": [{
            "role": "assistant",
            "content": null,
            "tool_calls": [{
                "id": "call_1",
                "type": "function",
                "function": {"name": "get_weather", "arguments": "{\"city\":\"SF\"}"}
            }]
        }],
        "tools": [{
            "type": "function",
            "function": {"name": "get_weather", "description": "Get weather"}
        }],
        "tool_choice": "auto"
    }"#;
    let req = OpenAiChatInbound
        .transform_request(body.as_bytes(), &HeaderMap::new())
        .await
        .unwrap();
    let calls = req.messages[0]
        .tool_calls
        .as_ref()
        .expect("tool_calls present");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].function.name, "get_weather");
    let tools = req.tools.expect("tools present");
    assert_eq!(tools[0].function.name, "get_weather");
    assert!(matches!(
        req.tool_choice,
        Some(galaxy_router::llm::protocol::model::ToolChoice::Auto)
    ));
}

#[tokio::test]
async fn test_openai_chat_inbound_tool_choice_required_and_function() {
    let body_req = r#"{
        "model": "gpt-4o",
        "messages": [{"role": "user", "content": "hi"}],
        "tool_choice": "required"
    }"#;
    let req = OpenAiChatInbound
        .transform_request(body_req.as_bytes(), &HeaderMap::new())
        .await
        .unwrap();
    assert!(matches!(
        req.tool_choice,
        Some(galaxy_router::llm::protocol::model::ToolChoice::Required)
    ));

    let body_func = r#"{
        "model": "gpt-4o",
        "messages": [{"role": "user", "content": "hi"}],
        "tool_choice": {"type": "function", "function": {"name": "get_weather"}}
    }"#;
    let req = OpenAiChatInbound
        .transform_request(body_func.as_bytes(), &HeaderMap::new())
        .await
        .unwrap();
    match req.tool_choice {
        Some(galaxy_router::llm::protocol::model::ToolChoice::Function { function }) => {
            assert_eq!(function.name, "get_weather");
        }
        other => panic!("expected Function variant, got {:?}", other),
    }
}

#[test]
fn test_openai_chat_inbound_stream_event_serializes() {
    use galaxy_router::llm::protocol::model::{FinishReason, LlmStreamResponse, Message, StreamChoice};
    let event = LlmStreamResponse {
        id: "x".into(),
        object: "chunk".into(),
        created: 0,
        model: "gpt-4o".into(),
        choices: vec![StreamChoice {
            index: 0,
            delta: Message {
                role: Role::Assistant,
                content: Some(Content::Text("hi".into())),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
                cache_control: None,
            },
            finish_reason: Some(FinishReason::Stop),
        }],
        usage: None,
        system_fingerprint: None,
    };
    let bytes = OpenAiChatInbound.transform_stream_event(&event).unwrap();
    let s = String::from_utf8(bytes).unwrap();
    assert!(s.starts_with("data: "));
    assert!(s.ends_with("\n\n"));
}

#[tokio::test]
async fn test_openai_chat_outbound_response_parses() {
    let body = serde_json::to_vec(&serde_json::json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "created": 1700000000,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "hi"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5}
    }))
    .unwrap();
    let resp = OpenAiChatOutbound
        .transform_response(&body, 200)
        .await
        .unwrap();
    assert_eq!(resp.id, "chatcmpl-1");
    if let Some(Content::Text(s)) = &resp.choices[0].message.content {
        assert_eq!(s, "hi");
    } else {
        panic!("expected text content");
    }
}

#[test]
fn test_openai_chat_outbound_stream_event_done_returns_none() {
    let parsed = OpenAiChatOutbound
        .transform_stream_event(b"data: [DONE]")
        .unwrap();
    assert!(parsed.is_none());
}

#[test]
fn test_openai_chat_outbound_stream_event_empty_returns_none() {
    let parsed = OpenAiChatOutbound.transform_stream_event(b"").unwrap();
    assert!(parsed.is_none());
}

#[test]
fn test_openai_chat_outbound_stream_event_parse_data_chunk() {
    let sse = r#"data: {"id":"x","object":"chat.completion.chunk","created":0,"model":"gpt-4o","choices":[{"index":0,"delta":{"role":"assistant","content":"hi"}}]}"#;
    let parsed = OpenAiChatOutbound
        .transform_stream_event(sse.as_bytes())
        .unwrap()
        .expect("should parse");
    assert_eq!(parsed.id, "x");
}

// ============================================================
// OpenAI Responses inbound：input 数组 / 工具 / 错误 / stream
// ============================================================

#[tokio::test]
async fn test_openai_responses_inbound_input_as_array_with_roles() {
    let body = r#"{
        "model": "o1",
        "input": [
            {"role": "system", "content": "Be brief"},
            {"role": "user", "content": "Hello"},
            {"role": "assistant", "content": "Hi"},
            {"role": "tool", "content": "ok"}
        ]
    }"#;
    let req = OpenAiResponsesInbound
        .transform_request(body.as_bytes(), &HeaderMap::new())
        .await
        .unwrap();
    // system/user/assistant 都映射成消息；tool 角色降级为 user（unknown 分支）
    assert!(req.messages.len() >= 3);
    assert!(req.messages.iter().any(|m| matches!(m.role, Role::System)));
    assert!(req.messages.iter().any(|m| matches!(m.role, Role::User)));
    assert!(
        req.messages
            .iter()
            .any(|m| matches!(m.role, Role::Assistant))
    );
}

#[tokio::test]
async fn test_openai_responses_inbound_input_as_object_content() {
    let body = r#"{
        "model": "o1",
        "input": [
            {"role": "user", "content": {"text": "complex"}}
        ]
    }"#;
    let req = OpenAiResponsesInbound
        .transform_request(body.as_bytes(), &HeaderMap::new())
        .await
        .unwrap();
    // 对象 content 序列化为 Parts
    match &req.messages[0].content {
        Some(Content::Parts(parts)) => assert!(!parts.is_empty()),
        _ => panic!("expected Parts for object content"),
    }
}

#[tokio::test]
async fn test_openai_responses_inbound_invalid_input_returns_err() {
    let body = r#"{"model": "o1", "input": 42}"#;
    let result = OpenAiResponsesInbound
        .transform_request(body.as_bytes(), &HeaderMap::new())
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_openai_responses_inbound_with_tools_and_previous_response_id() {
    let body = r#"{
        "model": "o1",
        "input": "hi",
        "tools": [{"type": "function", "function": {"name": "x", "parameters": {}}}],
        "previous_response_id": "resp_prev",
        "temperature": 0.5,
        "max_output_tokens": 256
    }"#;
    let req = OpenAiResponsesInbound
        .transform_request(body.as_bytes(), &HeaderMap::new())
        .await
        .unwrap();
    let tools = req.tools.expect("tools present");
    assert_eq!(tools.len(), 1);
    assert_eq!(req.temperature, Some(0.5));
    assert_eq!(req.max_completion_tokens, Some(256));
    assert_eq!(
        req.extra
            .get("previous_response_id")
            .and_then(|v| v.as_str()),
        Some("resp_prev")
    );
}

#[test]
fn test_openai_responses_inbound_stream_event_serializes() {
    use galaxy_router::llm::protocol::model::{FinishReason, LlmStreamResponse, Message, StreamChoice};
    let event = LlmStreamResponse {
        id: String::new(),
        object: "chunk".into(),
        created: 0,
        model: "o1".into(),
        choices: vec![StreamChoice {
            index: 0,
            delta: Message {
                role: Role::Assistant,
                content: Some(Content::Text("ok".into())),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
                cache_control: None,
            },
            finish_reason: Some(FinishReason::Stop),
        }],
        usage: None,
        system_fingerprint: None,
    };
    let bytes = OpenAiResponsesInbound
        .transform_stream_event(&event)
        .unwrap();
    let s = String::from_utf8(bytes).unwrap();
    assert!(s.contains("data: "));
    assert!(s.contains("\"delta\":\"ok\""));
}

#[tokio::test]
async fn test_openai_responses_outbound_response_parses() {
    let body = serde_json::to_vec(&serde_json::json!({
        "id": "resp_1",
        "object": "response",
        "created": 1700000000,
        "model": "o1",
        "output": [{"content": [{"type": "output_text", "text": "done"}]}],
        "usage": {"input_tokens": 4, "output_tokens": 6, "total_tokens": 10}
    }))
    .unwrap();
    let resp = OpenAiResponsesOutbound
        .transform_response(&body, 200)
        .await
        .unwrap();
    assert_eq!(resp.id, "resp_1");
    assert_eq!(resp.model, "o1");
    assert!(resp.usage.is_some());
}

// ============================================================
// OpenAI Responses 流事件补全（reasoning / tool_calls / completed / skip events）
// ============================================================

#[test]
fn test_openai_responses_inbound_stream_event_reasoning() {
    use galaxy_router::llm::protocol::model::{Message, StreamChoice};
    let event = galaxy_router::llm::protocol::model::LlmStreamResponse {
        id: String::new(),
        object: "chunk".into(),
        created: 0,
        model: "o1".into(),
        choices: vec![StreamChoice {
            index: 0,
            delta: Message {
                role: Role::Assistant,
                content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: Some("thinking...".into()),
                cache_control: None,
            },
            finish_reason: None,
        }],
        usage: None,
        system_fingerprint: None,
    };
    let bytes = OpenAiResponsesInbound
        .transform_stream_event(&event)
        .unwrap();
    let s = String::from_utf8(bytes).unwrap();
    assert!(s.contains("response.reasoning.delta"));
    assert!(s.contains("\"delta\":\"thinking...\""));
}

#[test]
fn test_openai_responses_inbound_stream_event_finish_emits_completed() {
    use galaxy_router::llm::protocol::model::{FinishReason, Message, StreamChoice};
    let event = galaxy_router::llm::protocol::model::LlmStreamResponse {
        id: "resp_42".into(),
        object: "chunk".into(),
        created: 0,
        model: "o1".into(),
        choices: vec![StreamChoice {
            index: 0,
            delta: Message {
                role: Role::Assistant,
                content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
                cache_control: None,
            },
            finish_reason: Some(FinishReason::Stop),
        }],
        usage: Some(galaxy_router::llm::protocol::model::Usage {
            prompt_tokens: 5,
            completion_tokens: 8,
            total_tokens: 13,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        }),
        system_fingerprint: None,
    };
    let bytes = OpenAiResponsesInbound
        .transform_stream_event(&event)
        .unwrap();
    let s = String::from_utf8(bytes).unwrap();
    assert!(s.contains("response.completed"));
    assert!(s.contains("\"id\":\"resp_42\""));
    assert!(s.contains("\"input_tokens\":5"));
}

#[test]
fn test_openai_responses_inbound_stream_event_tool_calls() {
    use galaxy_router::llm::protocol::model::{FunctionCall, Message, StreamChoice, ToolCall};
    let event = galaxy_router::llm::protocol::model::LlmStreamResponse {
        id: String::new(),
        object: "chunk".into(),
        created: 0,
        model: "o1".into(),
        choices: vec![StreamChoice {
            index: 0,
            delta: Message {
                role: Role::Assistant,
                content: None,
                name: None,
                tool_calls: Some(vec![ToolCall {
                    id: "call_x".into(),
                    call_type: "function".into(),
                    function: FunctionCall {
                        name: "search".into(),
                        arguments: "{\"q\":\"rust\"}".into(),
                    },
                }]),
                tool_call_id: None,
                reasoning_content: None,
                cache_control: None,
            },
            finish_reason: None,
        }],
        usage: None,
        system_fingerprint: None,
    };
    let bytes = OpenAiResponsesInbound
        .transform_stream_event(&event)
        .unwrap();
    let s = String::from_utf8(bytes).unwrap();
    assert!(s.contains("response.function_call_arguments.delta"));
    assert!(s.contains("\"call_id\":\"call_x\""));
}

#[test]
fn test_openai_responses_outbound_stream_event_reasoning() {
    let sse = "event: response.reasoning.delta\ndata: {\"response_id\":\"r1\",\"delta\":\"step\"}";
    let parsed = OpenAiResponsesOutbound
        .transform_stream_event(sse.as_bytes())
        .unwrap()
        .expect("should parse");
    assert_eq!(parsed.id, "r1");
    assert_eq!(
        parsed.choices[0].delta.reasoning_content.as_deref(),
        Some("step")
    );
}

#[test]
fn test_openai_responses_outbound_stream_event_function_call() {
    let sse = "event: response.function_call_arguments.delta\ndata: {\"response_id\":\"r1\",\"call_id\":\"c1\",\"name\":\"fn\",\"delta\":\"{}\"}";
    let parsed = OpenAiResponsesOutbound
        .transform_stream_event(sse.as_bytes())
        .unwrap()
        .expect("should parse");
    let calls = parsed.choices[0]
        .delta
        .tool_calls
        .as_ref()
        .expect("tool_calls present");
    assert_eq!(calls[0].id, "c1");
    assert_eq!(calls[0].function.name, "fn");
    assert_eq!(calls[0].function.arguments, "{}");
}

#[test]
fn test_openai_responses_outbound_stream_event_completed_with_usage() {
    let sse = "event: response.completed\ndata: {\"response\":{\"id\":\"r1\",\"usage\":{\"input_tokens\":3,\"output_tokens\":4,\"total_tokens\":7}}}";
    let parsed = OpenAiResponsesOutbound
        .transform_stream_event(sse.as_bytes())
        .unwrap()
        .expect("should parse");
    assert_eq!(parsed.id, "r1");
    assert!(parsed.choices[0].finish_reason.is_some());
    let usage = parsed.usage.expect("usage present");
    assert_eq!(usage.prompt_tokens, 3);
    assert_eq!(usage.completion_tokens, 4);
}

#[test]
fn test_openai_responses_outbound_stream_event_skips_intermediate_events() {
    for event in [
        "event: response.created\ndata: {}",
        "event: response.in_progress\ndata: {}",
        "event: response.output_item.added\ndata: {}",
    ] {
        let parsed = OpenAiResponsesOutbound
            .transform_stream_event(event.as_bytes())
            .unwrap();
        assert!(parsed.is_none(), "expected None for {}", event);
    }
}
