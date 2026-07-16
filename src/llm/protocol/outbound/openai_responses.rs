use async_trait::async_trait;

use super::{Outbound, OutboundError};
use crate::llm::protocol::model::*;

/// OpenAI Responses 出站转换器
pub struct OpenAiResponsesOutbound;

#[async_trait]
impl Outbound for OpenAiResponsesOutbound {
    fn transform_request(&self, request: &LlmRequest) -> Result<Vec<u8>, OutboundError> {
        // 将统一格式转换为 Responses 格式
        let input: Vec<serde_json::Value> = request
            .messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    _ => "user",
                };

                let content = match &m.content {
                    Some(Content::Text(text)) => serde_json::json!([
                        { "type": "input_text", "text": text }
                    ]),
                    Some(Content::Parts(parts)) => {
                        let content_parts: Vec<_> = parts
                            .iter()
                            .map(|p| match p {
                                ContentPart::Text { text, .. } => {
                                    serde_json::json!({ "type": "input_text", "text": text })
                                }
                                _ => serde_json::json!({ "type": "input_text", "text": "" }),
                            })
                            .collect();
                        serde_json::Value::Array(content_parts)
                    }
                    None => serde_json::json!([]),
                };

                serde_json::json!({
                    "role": role,
                    "content": content
                })
            })
            .collect();

        let mut body = serde_json::json!({
            "model": request.model,
            "input": input,
            "stream": request.stream,
            "temperature": request.temperature,
            "max_output_tokens": request.max_completion_tokens,
        });

        if let Some(tools) = &request.tools {
            let responses_tools: Vec<_> = tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "type": "function",
                        "name": t.function.name,
                        "description": t.function.description,
                        "parameters": t.function.parameters
                    })
                })
                .collect();
            body["tools"] = serde_json::json!(responses_tools);
        }

        if let Some(prev_id) = request
            .extra
            .get("previous_response_id")
            .and_then(|v| v.as_str())
        {
            body["previous_response_id"] = serde_json::json!(prev_id);
        }

        serde_json::to_vec(&body)
            .map_err(|e| OutboundError::TransformError(format!("序列化请求失败: {}", e)))
    }

    async fn transform_response(
        &self,
        body: &[u8],
        _status: u16,
    ) -> Result<LlmResponse, OutboundError> {
        // 解析 Responses 格式并转换为统一格式
        let response: serde_json::Value = serde_json::from_slice(body)
            .map_err(|e| OutboundError::ParseError(format!("解析 Responses 响应失败: {}", e)))?;

        let id = response["id"].as_str().unwrap_or("").to_string();
        let model = response["model"].as_str().unwrap_or("").to_string();
        let created = response["created_at"].as_i64().unwrap_or(0);

        let mut messages = vec![];
        if let Some(output) = response["output"].as_array() {
            for item in output {
                if item["type"] == "message"
                    && let Some(content) = item["content"].as_array()
                {
                    let text: String = content
                        .iter()
                        .filter_map(|c| {
                            if c["type"] == "output_text" {
                                c["text"].as_str()
                            } else {
                                None
                            }
                        })
                        .collect();

                    if !text.is_empty() {
                        messages.push(Choice {
                            index: 0,
                            message: Message {
                                role: Role::Assistant,
                                content: Some(Content::Text(text)),
                                name: None,
                                tool_calls: None,
                                tool_call_id: None,
                                reasoning_content: None,
                                cache_control: None,
                            },
                            finish_reason: Some(FinishReason::Stop),
                        });
                    }
                }
            }
        }

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
            created,
            model,
            choices: messages,
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
            OutboundError::ParseError(format!("解析 Responses 流式事件失败: {}", e))
        })?;

        match sse.event_type.as_str() {
            "response.output_text.delta" => {
                let delta = parsed["delta"].as_str().unwrap_or("");
                Ok(Some(LlmStreamResponse {
                    id: parsed["response_id"].as_str().unwrap_or("").to_string(),
                    object: "chat.completion.chunk".to_string(),
                    created: chrono::Utc::now().timestamp(),
                    model: String::new(),
                    choices: vec![StreamChoice {
                        index: 0,
                        delta: StreamDelta {
                            role: Some(Role::Assistant),
                            content: Some(Content::Text(delta.to_string())),
                            reasoning_content: None,
                            tool_calls: None,
                        },
                        finish_reason: None,
                    }],
                    usage: None,
                    system_fingerprint: None,
                }))
            }
            "response.reasoning.delta" => {
                let delta = parsed["delta"].as_str().unwrap_or("");
                Ok(Some(LlmStreamResponse {
                    id: parsed["response_id"].as_str().unwrap_or("").to_string(),
                    object: "chat.completion.chunk".to_string(),
                    created: chrono::Utc::now().timestamp(),
                    model: String::new(),
                    choices: vec![StreamChoice {
                        index: 0,
                        delta: StreamDelta {
                            role: Some(Role::Assistant),
                            content: None,
                            reasoning_content: Some(delta.to_string()),
                            tool_calls: None,
                        },
                        finish_reason: None,
                    }],
                    usage: None,
                    system_fingerprint: None,
                }))
            }
            "response.function_call_arguments.delta" => {
                let delta = parsed["delta"].as_str().unwrap_or("");
                let call_id = parsed["call_id"].as_str().unwrap_or("").to_string();
                let name = parsed["name"].as_str().unwrap_or("").to_string();
                Ok(Some(LlmStreamResponse {
                    id: parsed["response_id"].as_str().unwrap_or("").to_string(),
                    object: "chat.completion.chunk".to_string(),
                    created: chrono::Utc::now().timestamp(),
                    model: String::new(),
                    choices: vec![StreamChoice {
                        index: 0,
                        delta: StreamDelta {
                            role: Some(Role::Assistant),
                            content: None,
                            reasoning_content: None,
                            tool_calls: Some(vec![StreamToolCallDelta {
                                index: 0,
                                id: Some(call_id),
                                call_type: Some("function".to_string()),
                                function: Some(StreamFunctionDelta {
                                    name: Some(name),
                                    arguments: Some(delta.to_string()),
                                }),
                            }]),
                        },
                        finish_reason: None,
                    }],
                    usage: None,
                    system_fingerprint: None,
                }))
            }
            "response.completed" => {
                let mut usage = None;
                if let Some(resp) = parsed.get("response")
                    && let Some(u) = resp.get("usage")
                {
                    usage = Some(Usage {
                        prompt_tokens: u["input_tokens"].as_u64().unwrap_or(0) as u32,
                        completion_tokens: u["output_tokens"].as_u64().unwrap_or(0) as u32,
                        total_tokens: (u["input_tokens"].as_u64().unwrap_or(0)
                            + u["output_tokens"].as_u64().unwrap_or(0))
                            as u32,
                        prompt_tokens_details: None,
                        completion_tokens_details: None,
                    });
                }
                Ok(Some(LlmStreamResponse {
                    id: parsed["response"]["id"].as_str().unwrap_or("").to_string(),
                    object: "chat.completion.chunk".to_string(),
                    created: chrono::Utc::now().timestamp(),
                    model: String::new(),
                    choices: vec![StreamChoice {
                        index: 0,
                        delta: StreamDelta {
                            role: Some(Role::Assistant),
                            content: None,
                            reasoning_content: None,
                            tool_calls: None,
                        },
                        finish_reason: Some(FinishReason::Stop),
                    }],
                    usage,
                    system_fingerprint: None,
                }))
            }
            "response.created"
            | "response.in_progress"
            | "response.output_item.added"
            | "response.output_text.done"
            | "response.function_call_arguments.done" => Ok(None),
            _ => Ok(None),
        }
    }

    fn set_auth_header(&self, headers: &mut reqwest::header::HeaderMap, api_key: &str) {
        headers.insert(
            "Authorization",
            format!("Bearer {}", api_key)
                .parse()
                .expect("api_key validated at save"),
        );
    }
}
