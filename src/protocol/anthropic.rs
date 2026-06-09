use async_trait::async_trait;
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};

use super::inbound::{Inbound, InboundError};
use super::model::*;
use super::outbound::{Outbound, OutboundError};
use super::stream_converter::{StreamConvertError, StreamConverter};
use std::collections::HashMap;

/// Anthropic Messages 入站转换器
pub struct AnthropicInbound;

/// Anthropic Messages 出站转换器
pub struct AnthropicOutbound;

/// Anthropic 请求
#[derive(Debug, Deserialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    #[serde(default)]
    system: Option<serde_json::Value>,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    temperature: Option<f64>,
    #[serde(default)]
    top_p: Option<f64>,
    #[serde(default)]
    stream: Option<bool>,
    #[serde(default)]
    tools: Option<Vec<serde_json::Value>>,
}

/// Anthropic 消息
#[derive(Debug, Deserialize, Serialize)]
struct AnthropicMessage {
    role: String,
    content: serde_json::Value,
}

#[async_trait]
impl Inbound for AnthropicInbound {
    async fn transform_request(
        &self,
        body: &[u8],
        _headers: &HeaderMap,
    ) -> Result<LlmRequest, InboundError> {
        let request: AnthropicRequest = serde_json::from_slice(body)
            .map_err(|e| InboundError::ParseError(format!("解析 Anthropic 请求失败: {}", e)))?;

        let mut messages = vec![];

        // 处理 system 消息
        if let Some(system) = &request.system {
            let system_text = if let Some(s) = system.as_str() {
                s.to_string()
            } else if let Some(arr) = system.as_array() {
                arr.iter()
                    .filter_map(|item| {
                        if item["type"] == "text" {
                            item["text"].as_str().map(|s| s.to_string())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                return Err(InboundError::InvalidRequest(
                    "无效的 system 格式".to_string(),
                ));
            };

            messages.push(Message {
                role: Role::System,
                content: Some(Content::Text(system_text)),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
                cache_control: None,
            });
        }

        // 处理消息
        for msg in request.messages {
            let role = match msg.role.as_str() {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                _ => Role::User,
            };

            let mut reasoning_content = None;
            let content = if let Some(s) = msg.content.as_str() {
                Some(Content::Text(s.to_string()))
            } else if let Some(arr) = msg.content.as_array() {
                let parts: Vec<ContentPart> = arr
                    .iter()
                    .filter_map(|item| match item["type"].as_str()? {
                        "text" => Some(ContentPart::Text {
                            text: item["text"].as_str()?.to_string(),
                            cache_control: item
                                .get("cache_control")
                                .and_then(|cc| serde_json::from_value(cc.clone()).ok()),
                        }),
                        "image" => Some(ContentPart::ImageUrl {
                            image_url: ImageUrl {
                                url: item["source"]["data"].as_str()?.to_string(),
                                detail: None,
                            },
                            cache_control: None,
                        }),
                        "tool_use" => Some(ContentPart::ToolUse {
                            id: item["id"].as_str()?.to_string(),
                            name: item["name"].as_str()?.to_string(),
                            input: item["input"].clone(),
                        }),
                        "tool_result" => Some(ContentPart::ToolResult {
                            tool_call_id: item["tool_use_id"].as_str()?.to_string(),
                            content: item["content"].as_str().unwrap_or("").to_string(),
                        }),
                        "thinking" => {
                            if let Some(text) = item["thinking"].as_str() {
                                reasoning_content = Some(text.to_string());
                            }
                            None
                        }
                        _ => None,
                    })
                    .collect();

                Some(Content::Parts(parts))
            } else {
                None
            };

            messages.push(Message {
                role,
                content,
                name: None,
                tool_calls: None,
                tool_call_id: None,
                reasoning_content,
                cache_control: None,
            });
        }

        let tools = request.tools.map(|t| {
            t.into_iter()
                .filter_map(|tool| {
                    let name = tool["name"].as_str()?.to_string();
                    Some(Tool {
                        tool_type: "function".to_string(),
                        function: FunctionDefinition {
                            name,
                            description: tool["description"].as_str().map(String::from),
                            parameters: tool.get("input_schema").cloned(),
                        },
                    })
                })
                .collect()
        });

        let tool_choice = None;

        let mut extra = std::collections::HashMap::new();
        if let Some(system) = request.system {
            extra.insert("system".to_string(), system);
        }

        Ok(LlmRequest {
            model: request.model,
            messages,
            temperature: request.temperature,
            top_p: request.top_p,
            max_tokens: None,
            max_completion_tokens: request.max_tokens,
            stream: request.stream,
            tools,
            tool_choice,
            stop: None,
            reasoning_effort: None,
            extra,
        })
    }

    fn transform_response(&self, response: &LlmResponse) -> Result<Vec<u8>, InboundError> {
        let content: Vec<serde_json::Value> = response
            .choices
            .first()
            .map(|choice| {
                let mut parts = vec![];

                if let Some(reasoning) = &choice.message.reasoning_content {
                    parts.push(serde_json::json!({
                        "type": "thinking",
                        "thinking": reasoning
                    }));
                }

                if let Some(text_content) = &choice.message.content {
                    match text_content {
                        Content::Text(text) => {
                            parts.push(serde_json::json!({
                                "type": "text",
                                "text": text
                            }));
                        }
                        Content::Parts(content_parts) => {
                            for part in content_parts {
                                match part {
                                    ContentPart::Text { text, .. } => {
                                        parts.push(serde_json::json!({
                                            "type": "text",
                                            "text": text
                                        }));
                                    }
                                    ContentPart::ToolUse { id, name, input } => {
                                        parts.push(serde_json::json!({
                                            "type": "tool_use",
                                            "id": id,
                                            "name": name,
                                            "input": input
                                        }));
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }

                parts
            })
            .unwrap_or_default();

        let stop_reason = response.finish_reason().map(|fr| match fr {
            FinishReason::Stop => "end_turn",
            FinishReason::Length => "max_tokens",
            FinishReason::ToolCalls => "tool_use",
            _ => "end_turn",
        });

        let anthropic_response = serde_json::json!({
            "id": response.id,
            "type": "message",
            "role": "assistant",
            "content": content,
            "model": response.model,
            "stop_reason": stop_reason,
            "usage": {
                "input_tokens": response.usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(0),
                "output_tokens": response.usage.as_ref().map(|u| u.completion_tokens).unwrap_or(0)
            }
        });

        serde_json::to_vec(&anthropic_response)
            .map_err(|e| InboundError::TransformError(format!("序列化响应失败: {}", e)))
    }

    fn transform_stream_event(&self, event: &LlmStreamResponse) -> Result<Vec<u8>, InboundError> {
        let mut events = vec![];

        if let Some(choice) = event.first_choice() {
            if let Some(content) = &choice.delta.content
                && let Content::Text(text) = content
            {
                events.push(format!(
                    "event: content_block_delta\ndata: {}\n\n",
                    serde_json::json!({
                        "type": "content_block_delta",
                        "index": 0,
                        "delta": { "type": "text_delta", "text": text }
                    })
                ));
            }

            if choice.finish_reason.is_some() {
                events.push(format!(
                    "event: message_stop\ndata: {}\n\n",
                    serde_json::json!({ "type": "message_stop" })
                ));
            }
        }

        Ok(events.join("").into_bytes())
    }

    fn create_stream_converter(&self) -> Box<dyn StreamConverter> {
        Box::new(AnthropicStreamConverter::new())
    }
}

#[async_trait]
impl Outbound for AnthropicOutbound {
    fn transform_request(&self, request: &LlmRequest) -> Result<Vec<u8>, OutboundError> {
        let mut system = request.extra.get("system").cloned();
        let mut messages = vec![];

        for msg in &request.messages {
            match msg.role {
                Role::System => {
                    if system.is_none() {
                        system = msg.content.as_ref().map(|c| match c {
                            Content::Text(s) => serde_json::Value::String(s.clone()),
                            Content::Parts(parts) => {
                                let text_parts: Vec<_> = parts
                                    .iter()
                                    .filter_map(|p| {
                                        if let ContentPart::Text { text, .. } = p {
                                            Some(
                                                serde_json::json!({ "type": "text", "text": text }),
                                            )
                                        } else {
                                            None
                                        }
                                    })
                                    .collect();
                                serde_json::Value::Array(text_parts)
                            }
                        });
                    }
                }
                _ => {
                    let content = match &msg.content {
                        Some(Content::Text(text)) => serde_json::Value::String(text.clone()),
                        Some(Content::Parts(parts)) => {
                            let anthropic_parts: Vec<_> = parts
                                .iter()
                                .map(|p| match p {
                                    ContentPart::Text {
                                        text,
                                        cache_control,
                                    } => {
                                        let mut block = serde_json::json!({
                                            "type": "text",
                                            "text": text
                                        });
                                        if let Some(cc) = cache_control {
                                            block["cache_control"] =
                                                serde_json::to_value(cc).unwrap_or_default();
                                        }
                                        block
                                    }
                                    ContentPart::ToolUse { id, name, input } => serde_json::json!({
                                        "type": "tool_use",
                                        "id": id,
                                        "name": name,
                                        "input": input
                                    }),
                                    ContentPart::ToolResult {
                                        tool_call_id,
                                        content,
                                    } => serde_json::json!({
                                        "type": "tool_result",
                                        "tool_use_id": tool_call_id,
                                        "content": content
                                    }),
                                    _ => serde_json::json!({ "type": "text", "text": "" }),
                                })
                                .collect();
                            serde_json::Value::Array(anthropic_parts)
                        }
                        None => serde_json::Value::String(String::new()),
                    };

                    let role = match msg.role {
                        Role::User => "user",
                        Role::Assistant => "assistant",
                        _ => "user",
                    };

                    messages.push(serde_json::json!({
                        "role": role,
                        "content": content
                    }));
                }
            }
        }

        let mut body = serde_json::json!({
            "model": request.model,
            "messages": messages,
            "max_tokens": request.max_completion_tokens.unwrap_or(4096),
        });

        if let Some(system) = system {
            body["system"] = system;
        }
        if let Some(temp) = request.temperature {
            body["temperature"] = serde_json::json!(temp);
        }
        if let Some(stream) = request.stream {
            body["stream"] = serde_json::json!(stream);
        }
        if let Some(tools) = &request.tools {
            let anthropic_tools: Vec<_> = tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.function.name,
                        "description": t.function.description,
                        "input_schema": t.function.parameters
                    })
                })
                .collect();
            body["tools"] = serde_json::json!(anthropic_tools);
        }

        serde_json::to_vec(&body)
            .map_err(|e| OutboundError::TransformError(format!("序列化请求失败: {}", e)))
    }

    async fn transform_response(
        &self,
        body: &[u8],
        _status: u16,
    ) -> Result<LlmResponse, OutboundError> {
        let response: serde_json::Value = serde_json::from_slice(body)
            .map_err(|e| OutboundError::ParseError(format!("解析 Anthropic 响应失败: {}", e)))?;

        let id = response["id"].as_str().unwrap_or("").to_string();
        let model = response["model"].as_str().unwrap_or("").to_string();

        let mut message_content = vec![];
        let mut reasoning_content = None;
        if let Some(content) = response["content"].as_array() {
            for item in content {
                match item["type"].as_str() {
                    Some("text") => {
                        if let Some(text) = item["text"].as_str() {
                            message_content.push(ContentPart::Text {
                                text: text.to_string(),
                                cache_control: None,
                            });
                        }
                    }
                    Some("tool_use") => {
                        message_content.push(ContentPart::ToolUse {
                            id: item["id"].as_str().unwrap_or("").to_string(),
                            name: item["name"].as_str().unwrap_or("").to_string(),
                            input: item["input"].clone(),
                        });
                    }
                    Some("thinking") if reasoning_content.is_none() => {
                        reasoning_content = item["thinking"].as_str().map(String::from);
                    }
                    _ => {}
                }
            }
        }

        let content = if message_content.len() == 1 {
            if let ContentPart::Text { text, .. } = &message_content[0] {
                Some(Content::Text(text.clone()))
            } else {
                Some(Content::Parts(message_content))
            }
        } else if message_content.is_empty() {
            None
        } else {
            Some(Content::Parts(message_content))
        };

        let finish_reason = response["stop_reason"].as_str().map(|sr| match sr {
            "end_turn" => FinishReason::Stop,
            "max_tokens" => FinishReason::Length,
            "tool_use" => FinishReason::ToolCalls,
            _ => FinishReason::Stop,
        });

        let usage = response["usage"].as_object().map(|u| Usage {
            prompt_tokens: u["input_tokens"].as_u64().unwrap_or(0) as u32,
            completion_tokens: u["output_tokens"].as_u64().unwrap_or(0) as u32,
            total_tokens: (u["input_tokens"].as_u64().unwrap_or(0)
                + u["output_tokens"].as_u64().unwrap_or(0)) as u32,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        });

        Ok(LlmResponse {
            id,
            object: "chat.completion".to_string(),
            created: chrono::Utc::now().timestamp(),
            model,
            choices: vec![Choice {
                index: 0,
                message: Message {
                    role: Role::Assistant,
                    content,
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                    reasoning_content,
                    cache_control: None,
                },
                finish_reason,
            }],
            usage,
            system_fingerprint: None,
        })
    }

    fn transform_stream_event(
        &self,
        event: &[u8],
    ) -> Result<Option<LlmStreamResponse>, OutboundError> {
        let text = String::from_utf8_lossy(event);
        let text = text.trim();

        if text.is_empty() {
            return Ok(None);
        }

        let sse = parse_sse_event(text);
        if sse.data.is_empty() {
            return Ok(None);
        }

        let parsed: serde_json::Value = serde_json::from_str(&sse.data).map_err(|e| {
            OutboundError::ParseError(format!("解析 Anthropic 流式事件失败: {}", e))
        })?;

        match sse.event_type.as_str() {
            "message_start" => {
                let id = parsed["message"]["id"].as_str().unwrap_or("").to_string();
                let model = parsed["message"]["model"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                Ok(Some(LlmStreamResponse {
                    id,
                    object: "chat.completion.chunk".to_string(),
                    created: chrono::Utc::now().timestamp(),
                    model,
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
                        finish_reason: None,
                    }],
                    usage: None,
                    system_fingerprint: None,
                }))
            }
            "content_block_delta" => {
                let delta_type = parsed["delta"]["type"].as_str().unwrap_or("");
                match delta_type {
                    "thinking_delta" => {
                        let thinking = parsed["delta"]["thinking"].as_str().unwrap_or("");
                        Ok(Some(LlmStreamResponse {
                            id: String::new(),
                            object: "chat.completion.chunk".to_string(),
                            created: chrono::Utc::now().timestamp(),
                            model: String::new(),
                            choices: vec![StreamChoice {
                                index: 0,
                                delta: Message {
                                    role: Role::Assistant,
                                    content: None,
                                    name: None,
                                    tool_calls: None,
                                    tool_call_id: None,
                                    reasoning_content: Some(thinking.to_string()),
                                    cache_control: None,
                                },
                                finish_reason: None,
                            }],
                            usage: None,
                            system_fingerprint: None,
                        }))
                    }
                    _ => {
                        let delta_text = parsed["delta"]["text"].as_str().unwrap_or("");
                        Ok(Some(LlmStreamResponse {
                            id: String::new(),
                            object: "chat.completion.chunk".to_string(),
                            created: chrono::Utc::now().timestamp(),
                            model: String::new(),
                            choices: vec![StreamChoice {
                                index: 0,
                                delta: Message {
                                    role: Role::Assistant,
                                    content: Some(Content::Text(delta_text.to_string())),
                                    name: None,
                                    tool_calls: None,
                                    tool_call_id: None,
                                    reasoning_content: None,
                                    cache_control: None,
                                },
                                finish_reason: None,
                            }],
                            usage: None,
                            system_fingerprint: None,
                        }))
                    }
                }
            }
            "message_delta" => {
                let finish_reason = parsed["delta"]["stop_reason"].as_str().map(|sr| match sr {
                    "end_turn" => FinishReason::Stop,
                    "max_tokens" => FinishReason::Length,
                    "tool_use" => FinishReason::ToolCalls,
                    _ => FinishReason::Stop,
                });
                Ok(Some(LlmStreamResponse {
                    id: String::new(),
                    object: "chat.completion.chunk".to_string(),
                    created: chrono::Utc::now().timestamp(),
                    model: String::new(),
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
                        finish_reason,
                    }],
                    usage: None,
                    system_fingerprint: None,
                }))
            }
            "message_stop" => Ok(None),
            _ => Ok(None),
        }
    }

    fn set_auth_header(&self, headers: &mut reqwest::header::HeaderMap, api_key: &str) {
        headers.insert(
            "x-api-key",
            api_key.parse().expect("api_key validated at save"),
        );
        headers.insert(
            "anthropic-version",
            "2023-06-01".parse().expect("static value"),
        );
    }
}

// ============================================================================
// AnthropicStreamConverter — 有状态的流式转换器
// 参考 AxonHub anthropicInboundStream 实现
// ============================================================================

/// Tool call 追踪状态
#[allow(dead_code)]
struct AnthropicToolCallState {
    id: String,
    name: String,
    arguments: String,
}

/// Anthropic Messages 有状态流式转换器
///
/// 将统一的 `LlmStreamResponse` 流转换为 Anthropic SSE 事件序列，
/// 包含完整 content block 生命周期：message_start → content_block_start →
/// content_block_delta → content_block_stop → message_delta → message_stop
pub struct AnthropicStreamConverter {
    // 生命周期标记
    has_started: bool,
    has_thinking_content_started: bool,
    has_text_content_started: bool,
    has_tool_content_started: bool,
    has_finished: bool,
    message_stopped: bool,

    // 元数据（从首个 chunk 提取，后续复用）
    message_id: String,
    model: String,

    // 内容块索引
    content_index: usize,

    // 当前 tool call 追踪
    current_tool_index: usize,
    tool_calls: HashMap<usize, AnthropicToolCallState>,

    // Usage（Anthropic 分 message_start 和 message_delta 两次发）
    input_usage: Option<Usage>,

    // 去重
    last_event_type: String,
}

impl Default for AnthropicStreamConverter {
    fn default() -> Self {
        Self::new()
    }
}

impl AnthropicStreamConverter {
    pub fn new() -> Self {
        Self {
            has_started: false,
            has_thinking_content_started: false,
            has_text_content_started: false,
            has_tool_content_started: false,
            has_finished: false,
            message_stopped: false,
            message_id: String::new(),
            model: String::new(),
            content_index: 0,
            current_tool_index: 0,
            tool_calls: HashMap::new(),
            input_usage: None,
            last_event_type: String::new(),
        }
    }

    /// 格式化一个 SSE 事件
    fn sse_event(event_type: &str, data: serde_json::Value) -> Vec<u8> {
        format!("event: {}\ndata: {}\n\n", event_type, data).into_bytes()
    }

    /// 去重检查：防止重复 content_block_stop
    fn should_emit(&mut self, event_type: &str) -> bool {
        if self.last_event_type == "content_block_stop" && event_type == "content_block_stop" {
            return false;
        }
        self.last_event_type = event_type.to_string();
        true
    }

    /// 发送 message_start（仅首次）
    fn emit_message_start(&mut self, event: &LlmStreamResponse) -> Vec<Vec<u8>> {
        if self.has_started {
            return vec![];
        }
        self.has_started = true;

        // 从首个 chunk 提取元数据
        if self.message_id.is_empty() && !event.id.is_empty() {
            self.message_id = event.id.clone();
        }
        if self.model.is_empty() && !event.model.is_empty() {
            self.model = event.model.clone();
        }
        if event.usage.is_some() {
            self.input_usage = event.usage.clone();
        }

        let usage = serde_json::json!({
            "input_tokens": self.input_usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(1),
            "output_tokens": self.input_usage.as_ref().map(|u| u.completion_tokens).unwrap_or(1)
        });

        vec![Self::sse_event(
            "message_start",
            serde_json::json!({
                "type": "message_start",
                "message": {
                    "id": self.message_id,
                    "type": "message",
                    "role": "assistant",
                    "content": [],
                    "model": self.model,
                    "stop_reason": null,
                    "usage": usage
                }
            }),
        )]
    }

    /// 关闭 thinking block
    /// 在 content_block_stop 之前补发 signature_delta 事件（如果有 pending signature）
    fn close_thinking_block(&mut self) -> Vec<Vec<u8>> {
        if !self.has_thinking_content_started {
            return vec![];
        }
        self.has_thinking_content_started = false;

        let mut events = vec![];
        if self.should_emit("content_block_stop") {
            events.push(Self::sse_event(
                "content_block_stop",
                serde_json::json!({
                    "type": "content_block_stop",
                    "index": self.content_index
                }),
            ));
        }
        self.content_index += 1;
        events
    }

    /// 关闭 text block
    fn close_text_block(&mut self) -> Vec<Vec<u8>> {
        if !self.has_text_content_started {
            return vec![];
        }
        self.has_text_content_started = false;

        let mut events = vec![];
        if self.should_emit("content_block_stop") {
            events.push(Self::sse_event(
                "content_block_stop",
                serde_json::json!({
                    "type": "content_block_stop",
                    "index": self.content_index
                }),
            ));
        }
        self.content_index += 1;
        events
    }

    /// 关闭 tool block
    fn close_tool_block(&mut self) -> Vec<Vec<u8>> {
        if !self.has_tool_content_started {
            return vec![];
        }
        self.has_tool_content_started = false;

        let mut events = vec![];
        if self.should_emit("content_block_stop") {
            events.push(Self::sse_event(
                "content_block_stop",
                serde_json::json!({
                    "type": "content_block_stop",
                    "index": self.content_index
                }),
            ));
        }
        self.content_index += 1;
        events
    }

    /// 处理 reasoning_content（thinking block）
    fn handle_thinking(&mut self, thinking: &str) -> Vec<Vec<u8>> {
        let mut events = vec![];

        // 关闭其他打开的 block
        events.extend(self.close_tool_block());
        events.extend(self.close_text_block());

        // 启动 thinking block
        if !self.has_thinking_content_started {
            self.has_thinking_content_started = true;
            events.push(Self::sse_event(
                "content_block_start",
                serde_json::json!({
                    "type": "content_block_start",
                    "index": self.content_index,
                    "content_block": { "type": "thinking" }
                }),
            ));
        }

        // 发送 thinking delta
        events.push(Self::sse_event(
            "content_block_delta",
            serde_json::json!({
                "type": "content_block_delta",
                "index": self.content_index,
                "delta": { "type": "thinking_delta", "thinking": thinking }
            }),
        ));

        events
    }

    /// 处理 text content
    fn handle_text(&mut self, text: &str) -> Vec<Vec<u8>> {
        let mut events = vec![];

        // 关闭 thinking 和 tool block
        events.extend(self.close_thinking_block());
        events.extend(self.close_tool_block());

        // 启动 text block
        if !self.has_text_content_started {
            self.has_text_content_started = true;
            events.push(Self::sse_event(
                "content_block_start",
                serde_json::json!({
                    "type": "content_block_start",
                    "index": self.content_index,
                    "content_block": { "type": "text" }
                }),
            ));
        }

        // 发送 text delta
        events.push(Self::sse_event(
            "content_block_delta",
            serde_json::json!({
                "type": "content_block_delta",
                "index": self.content_index,
                "delta": { "type": "text_delta", "text": text }
            }),
        ));

        events
    }

    /// 处理 tool_calls
    fn handle_tool_calls(&mut self, tool_calls: &[ToolCall]) -> Vec<Vec<u8>> {
        let mut events = vec![];

        // 关闭 thinking 和 text block
        events.extend(self.close_thinking_block());
        events.extend(self.close_text_block());

        for tc in tool_calls {
            // 用 index 追踪（streaming 中同一个 tool call 会多次出现）
            let idx = self.current_tool_index;

            let is_new = !self.tool_calls.contains_key(&idx);
            if is_new {
                // 关闭之前的 tool block
                if self.has_tool_content_started {
                    events.extend(self.close_tool_block());
                }
                self.has_tool_content_started = true;

                self.tool_calls.insert(
                    idx,
                    AnthropicToolCallState {
                        id: tc.id.clone(),
                        name: tc.function.name.clone(),
                        arguments: String::new(),
                    },
                );

                events.push(Self::sse_event(
                    "content_block_start",
                    serde_json::json!({
                        "type": "content_block_start",
                        "index": self.content_index,
                        "content_block": {
                            "type": "tool_use",
                            "id": tc.id,
                            "name": tc.function.name,
                            "input": {}
                        }
                    }),
                ));

                // 如果有初始 arguments，发送 delta
                if !tc.function.arguments.is_empty() {
                    events.push(Self::sse_event("content_block_delta", serde_json::json!({
                        "type": "content_block_delta",
                        "index": self.content_index,
                        "delta": { "type": "input_json_delta", "partial_json": tc.function.arguments }
                    })));
                    if let Some(state) = self.tool_calls.get_mut(&idx) {
                        state.arguments.push_str(&tc.function.arguments);
                    }
                }

                self.current_tool_index += 1;
            } else if let Some(state) = self.tool_calls.get_mut(&idx) {
                // 追加 arguments
                state.arguments.push_str(&tc.function.arguments);
                if !tc.function.arguments.is_empty() {
                    events.push(Self::sse_event("content_block_delta", serde_json::json!({
                        "type": "content_block_delta",
                        "index": self.content_index,
                        "delta": { "type": "input_json_delta", "partial_json": tc.function.arguments }
                    })));
                }
            }
        }

        events
    }

    /// 处理 finish_reason
    fn handle_finish(&mut self, event: &LlmStreamResponse) -> Vec<Vec<u8>> {
        if self.has_finished {
            return vec![];
        }
        self.has_finished = true;

        let mut events = vec![];

        // 关闭 thinking block
        events.extend(self.close_thinking_block());

        // 关闭当前 content block
        if self.has_tool_content_started {
            events.extend(self.close_tool_block());
        } else if self.has_text_content_started {
            events.extend(self.close_text_block());
        }

        // 映射 finish_reason
        let stop_reason = event
            .first_choice()
            .and_then(|c| c.finish_reason.as_ref())
            .map(|fr| match fr {
                FinishReason::Stop => "end_turn",
                FinishReason::Length => "max_tokens",
                FinishReason::ToolCalls => "tool_use",
                _ => "end_turn",
            })
            .unwrap_or("end_turn");

        // 发送 message_delta + message_stop
        if !self.message_stopped {
            self.message_stopped = true;

            let usage = if let Some(u) = &event.usage {
                serde_json::json!({
                    "output_tokens": u.completion_tokens
                })
            } else {
                serde_json::json!({ "output_tokens": 0 })
            };

            events.push(Self::sse_event(
                "message_delta",
                serde_json::json!({
                    "type": "message_delta",
                    "delta": {
                        "stop_reason": stop_reason
                    },
                    "usage": usage
                }),
            ));
            events.push(Self::sse_event(
                "message_stop",
                serde_json::json!({
                    "type": "message_stop"
                }),
            ));
        }

        events
    }
}

impl StreamConverter for AnthropicStreamConverter {
    fn convert(&mut self, event: &LlmStreamResponse) -> Result<Vec<Vec<u8>>, StreamConvertError> {
        let mut all_events = Vec::new();

        // 确保已发送 message_start
        all_events.extend(self.emit_message_start(event));

        // 更新元数据
        if self.message_id.is_empty() && !event.id.is_empty() {
            self.message_id = event.id.clone();
        }
        if self.model.is_empty() && !event.model.is_empty() {
            self.model = event.model.clone();
        }
        if event.usage.is_some() {
            self.input_usage = event.usage.clone();
        }

        if let Some(choice) = event.first_choice() {
            // 处理 reasoning_content（thinking block）
            if let Some(reasoning) = &choice.delta.reasoning_content
                && !reasoning.is_empty()
            {
                all_events.extend(self.handle_thinking(reasoning));
            }

            // 处理 text content
            if let Some(content) = &choice.delta.content
                && let Content::Text(text) = content
                && !text.is_empty()
            {
                all_events.extend(self.handle_text(text));
            }

            // 处理 tool_calls
            if let Some(tool_calls) = &choice.delta.tool_calls
                && !tool_calls.is_empty()
            {
                all_events.extend(self.handle_tool_calls(tool_calls));
            }

            // 处理 finish_reason
            if choice.finish_reason.is_some() {
                all_events.extend(self.handle_finish(event));
            }
        }

        // 如果有 usage 但还没 finished
        if event.usage.is_some() && self.has_finished && !self.message_stopped {
            all_events.extend(self.handle_finish(event));
        }

        Ok(all_events)
    }

    fn finish(&mut self) -> Result<Vec<Vec<u8>>, StreamConvertError> {
        if self.message_stopped {
            return Ok(vec![]);
        }
        Ok(vec![])
    }
}
