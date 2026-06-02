use async_trait::async_trait;
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};

use super::inbound::{Inbound, InboundError};
use super::model::*;
use super::outbound::{Outbound, OutboundError};

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
