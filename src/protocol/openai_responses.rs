use async_trait::async_trait;
use axum::http::HeaderMap;
use serde::Deserialize;

use super::inbound::{Inbound, InboundError};
use super::model::*;
use super::outbound::{Outbound, OutboundError};

/// OpenAI Responses 入站转换器
pub struct OpenAiResponsesInbound;

/// OpenAI Responses 出站转换器
pub struct OpenAiResponsesOutbound;

/// OpenAI Responses 请求
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OpenAiResponsesRequest {
    model: String,
    input: serde_json::Value,
    #[serde(default)]
    stream: Option<bool>,
    #[serde(default)]
    temperature: Option<f64>,
    #[serde(default)]
    max_output_tokens: Option<u32>,
    #[serde(default)]
    tools: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    previous_response_id: Option<String>,
}

#[async_trait]
impl Inbound for OpenAiResponsesInbound {
    async fn transform_request(
        &self,
        body: &[u8],
        _headers: &HeaderMap,
    ) -> Result<LlmRequest, InboundError> {
        let request: OpenAiResponsesRequest = serde_json::from_slice(body).map_err(|e| {
            InboundError::ParseError(format!("解析 OpenAI Responses 请求失败: {}", e))
        })?;

        // 将 input 转换为 messages
        let messages = if let Some(input_str) = request.input.as_str() {
            // 简单文本输入
            vec![Message {
                role: Role::User,
                content: Some(Content::Text(input_str.to_string())),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
                cache_control: None,
            }]
        } else if let Some(input_array) = request.input.as_array() {
            // 数组输入
            input_array
                .iter()
                .filter_map(|item| {
                    if let Some(item_obj) = item.as_object() {
                        let role = item_obj.get("role")?.as_str()?;
                        let role = match role {
                            "system" => Role::System,
                            "user" => Role::User,
                            "assistant" => Role::Assistant,
                            _ => Role::User,
                        };

                        let content = item_obj.get("content").map(|c| {
                            if let Some(s) = c.as_str() {
                                Content::Text(s.to_string())
                            } else {
                                Content::Parts(vec![ContentPart::Text {
                                    text: serde_json::to_string(c).unwrap_or_default(),
                                    cache_control: None,
                                }])
                            }
                        });

                        Some(Message {
                            role,
                            content,
                            name: None,
                            tool_calls: None,
                            tool_call_id: None,
                            reasoning_content: None,
                            cache_control: None,
                        })
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            return Err(InboundError::InvalidRequest(
                "无效的 input 格式".to_string(),
            ));
        };

        let tools = request.tools.map(|t| {
            t.into_iter()
                .filter_map(|tool| {
                    if tool["type"] != "function" {
                        return None;
                    }
                    let func = tool.get("function")?;
                    let name = func["name"].as_str()?.to_string();
                    Some(Tool {
                        tool_type: "function".to_string(),
                        function: FunctionDefinition {
                            name,
                            description: func["description"].as_str().map(String::from),
                            parameters: func.get("parameters").cloned(),
                        },
                    })
                })
                .collect()
        });

        let mut extra = std::collections::HashMap::new();
        if let Some(prev_id) = request.previous_response_id {
            extra.insert(
                "previous_response_id".to_string(),
                serde_json::json!(prev_id),
            );
        }

        Ok(LlmRequest {
            model: request.model,
            messages,
            temperature: request.temperature,
            top_p: None,
            max_tokens: None,
            max_completion_tokens: request.max_output_tokens,
            stream: request.stream,
            tools,
            tool_choice: None,
            stop: None,
            reasoning_effort: None,
            extra,
        })
    }

    fn transform_response(&self, response: &LlmResponse) -> Result<Vec<u8>, InboundError> {
        // 将 OpenAI Chat 格式转换为 Responses 格式
        let output = response
            .choices
            .first()
            .map(|choice| {
                let mut items = vec![];

                if let Some(reasoning) = &choice.message.reasoning_content {
                    items.push(serde_json::json!({
                        "type": "reasoning",
                        "summary": [{ "type": "summary_text", "text": reasoning }]
                    }));
                }

                if let Some(content) = &choice.message.content {
                    match content {
                        Content::Text(text) => {
                            items.push(serde_json::json!({
                                "type": "message",
                                "role": "assistant",
                                "content": [{ "type": "output_text", "text": text }]
                            }));
                        }
                        Content::Parts(parts) => {
                            let text_parts: Vec<_> = parts.iter().filter_map(|p| {
                            if let ContentPart::Text { text, .. } = p {
                                Some(serde_json::json!({ "type": "output_text", "text": text }))
                            } else {
                                None
                            }
                        }).collect();

                            if !text_parts.is_empty() {
                                items.push(serde_json::json!({
                                    "type": "message",
                                    "role": "assistant",
                                    "content": text_parts
                                }));
                            }
                        }
                    }
                }

                if let Some(tool_calls) = &choice.message.tool_calls {
                    for tc in tool_calls {
                        items.push(serde_json::json!({
                            "type": "function_call",
                            "id": tc.id,
                            "name": tc.function.name,
                            "arguments": tc.function.arguments
                        }));
                    }
                }

                items
            })
            .unwrap_or_default();

        let responses_format = serde_json::json!({
            "id": response.id,
            "object": "response",
            "created_at": response.created,
            "model": response.model,
            "output": output,
            "usage": response.usage,
            "status": "completed"
        });

        serde_json::to_vec(&responses_format)
            .map_err(|e| InboundError::TransformError(format!("序列化响应失败: {}", e)))
    }

    fn transform_stream_event(&self, event: &LlmStreamResponse) -> Result<Vec<u8>, InboundError> {
        let mut events = vec![];

        if let Some(choice) = event.first_choice() {
            if let Some(reasoning) = &choice.delta.reasoning_content
                && !reasoning.is_empty()
            {
                events.push(format!(
                    "event: response.reasoning.delta\ndata: {}\n\n",
                    serde_json::json!({
                        "type": "response.reasoning.delta",
                        "output_index": 0,
                        "delta": reasoning
                    })
                ));
            }

            if let Some(content) = &choice.delta.content
                && let Content::Text(text) = content
                && !text.is_empty()
            {
                events.push(format!(
                    "event: response.output_text.delta\ndata: {}\n\n",
                    serde_json::json!({
                        "type": "response.output_text.delta",
                        "output_index": 0,
                        "content_index": 0,
                        "delta": text
                    })
                ));
            }

            if let Some(tool_calls) = &choice.delta.tool_calls {
                for tc in tool_calls {
                    events.push(format!(
                        "event: response.function_call_arguments.delta\ndata: {}\n\n",
                        serde_json::json!({
                            "type": "response.function_call_arguments.delta",
                            "output_index": 0,
                            "call_id": tc.id,
                            "name": tc.function.name,
                            "delta": tc.function.arguments
                        })
                    ));
                }
            }

            if choice.finish_reason.is_some() {
                let mut response_obj = serde_json::json!({
                    "id": event.id,
                    "status": "completed"
                });
                if let Some(usage) = &event.usage {
                    response_obj["usage"] = serde_json::json!({
                        "input_tokens": usage.prompt_tokens,
                        "output_tokens": usage.completion_tokens,
                        "total_tokens": usage.total_tokens,
                    });
                }
                events.push(format!(
                    "event: response.completed\ndata: {}\n\n",
                    serde_json::json!({
                        "type": "response.completed",
                        "response": response_obj
                    })
                ));
            }
        }

        Ok(events.join("").into_bytes())
    }

    fn protocol_name(&self) -> &'static str {
        "openai_responses"
    }
}

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
                        delta: Message {
                            role: Role::Assistant,
                            content: Some(Content::Text(delta.to_string())),
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
            "response.reasoning.delta" => {
                let delta = parsed["delta"].as_str().unwrap_or("");
                Ok(Some(LlmStreamResponse {
                    id: parsed["response_id"].as_str().unwrap_or("").to_string(),
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
                            reasoning_content: Some(delta.to_string()),
                            cache_control: None,
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
                        delta: Message {
                            role: Role::Assistant,
                            content: None,
                            name: None,
                            tool_calls: Some(vec![ToolCall {
                                id: call_id,
                                call_type: "function".to_string(),
                                function: FunctionCall {
                                    name,
                                    arguments: delta.to_string(),
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

    fn api_format(&self) -> &'static str {
        "openai_responses"
    }

    fn request_path(&self) -> &'static str {
        "/v1/responses"
    }

    fn set_auth_header(&self, headers: &mut reqwest::header::HeaderMap, api_key: &str) {
        headers.insert(
            "Authorization",
            format!("Bearer {}", api_key).parse().unwrap(),
        );
    }
}
