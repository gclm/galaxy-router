use async_trait::async_trait;
use axum::http::HeaderMap;
use serde::Deserialize;

use super::openai_responses_converter::ResponsesStreamConverter;
use super::{Inbound, InboundError};
use crate::llm::protocol::model::*;
use crate::llm::protocol::stream_converter::StreamConverter;

/// OpenAI Responses 入站转换器
pub struct OpenAiResponsesInbound;

/// OpenAI Responses 请求
/// OpenAI Responses 请求
#[derive(Debug, Deserialize)]
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
                    let id = tc.id.clone().unwrap_or_default();
                    let name = tc
                        .function
                        .as_ref()
                        .and_then(|f| f.name.clone())
                        .unwrap_or_default();
                    let arguments = tc
                        .function
                        .as_ref()
                        .and_then(|f| f.arguments.clone())
                        .unwrap_or_default();
                    events.push(format!(
                        "event: response.function_call_arguments.delta\ndata: {}\n\n",
                        serde_json::json!({
                            "type": "response.function_call_arguments.delta",
                            "output_index": 0,
                            "call_id": id,
                            "name": name,
                            "delta": arguments
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

    fn create_stream_converter(&self) -> Box<dyn StreamConverter> {
        Box::new(ResponsesStreamConverter::new())
    }
}
