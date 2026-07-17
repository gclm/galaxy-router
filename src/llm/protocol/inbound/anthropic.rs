use async_trait::async_trait;
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};

use super::anthropic_converter::AnthropicStreamConverter;
use super::{Inbound, InboundError};
use crate::llm::protocol::model::*;
use crate::llm::protocol::stream_converter::StreamConverter;

/// Anthropic Messages 入站转换器
pub struct AnthropicInbound;

/// Anthropic 请求
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
pub(crate) struct AnthropicMessage {
    pub(crate) role: String,
    pub(crate) content: serde_json::Value,
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
                        "image" => {
                            // 兼容 Anthropic source 的 base64 / url 两种类型；
                            // base64 统一编码成 data URL 存入 IR，url 直接透传
                            let src = &item["source"];
                            let url = match src["type"].as_str() {
                                Some("url") => src["url"].as_str()?.to_string(),
                                Some("base64") => {
                                    let media_type = src["media_type"].as_str().unwrap_or("image/png");
                                    let data = src["data"].as_str()?;
                                    crate::llm::protocol::multimodal::build_data_url(media_type, data)
                                }
                                _ => src["data"].as_str()?.to_string(),
                            };
                            Some(ContentPart::ImageUrl {
                                image_url: ImageUrl { url, detail: None },
                                cache_control: None,
                            })
                        }
                        "tool_use" => Some(ContentPart::ToolUse {
                            id: item["id"].as_str()?.to_string(),
                            name: item["name"].as_str()?.to_string(),
                            input: item["input"].clone(),
                        }),
                        "tool_result" => {
                            // content 可为字符串或 block 数组；数组取各 block 文本拼接
                            let content = item["content"].as_str().map(String::from).unwrap_or_else(|| {
                                item["content"]
                                    .as_array()
                                    .map(|arr| {
                                        arr.iter()
                                            .filter_map(|b| b["text"].as_str())
                                            .collect::<Vec<_>>()
                                            .join("")
                                    })
                                    .unwrap_or_default()
                            });
                            Some(ContentPart::ToolResult {
                                tool_call_id: item["tool_use_id"].as_str()?.to_string(),
                                content,
                            })
                        }
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

    fn create_stream_converter(&self) -> Box<dyn StreamConverter> {
        Box::new(AnthropicStreamConverter::new())
    }
}
