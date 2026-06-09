use async_trait::async_trait;

use super::{Outbound, OutboundError};
use crate::protocol::model::*;

/// Anthropic Messages 出站转换器
pub struct AnthropicOutbound;

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

