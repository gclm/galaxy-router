use async_trait::async_trait;
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};

use super::{Inbound, InboundError};
use crate::protocol::model::*;
use crate::protocol::stream_converter::{SimpleStreamConverter, StreamConverter};

/// OpenAI Chat Completions 入站转换器
pub struct OpenAiChatInbound;

/// OpenAI Chat 请求
#[derive(Debug, Deserialize)]
struct OpenAiChatRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    #[serde(default)]
    temperature: Option<f64>,
    #[serde(default)]
    top_p: Option<f64>,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    max_completion_tokens: Option<u32>,
    #[serde(default)]
    stream: Option<bool>,
    #[serde(default)]
    tools: Option<Vec<OpenAiTool>>,
    #[serde(default)]
    tool_choice: Option<serde_json::Value>,
    #[serde(default)]
    stop: Option<Vec<String>>,
}

/// OpenAI 消息
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct OpenAiMessage {
    pub(crate) role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) content: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_calls: Option<Vec<OpenAiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_call_id: Option<String>,
}

/// OpenAI 工具调用
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct OpenAiToolCall {
    pub(crate) id: String,
    #[serde(rename = "type")]
    pub(crate) call_type: String,
    pub(crate) function: OpenAiFunctionCall,
}

/// OpenAI 函数调用
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct OpenAiFunctionCall {
    pub(crate) name: String,
    pub(crate) arguments: String,
}

/// OpenAI 工具
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct OpenAiTool {
    #[serde(rename = "type")]
    pub(crate) tool_type: String,
    pub(crate) function: OpenAiFunctionDefinition,
}

/// OpenAI 函数定义
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct OpenAiFunctionDefinition {
    pub(crate) name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) parameters: Option<serde_json::Value>,
}

#[async_trait]
impl Inbound for OpenAiChatInbound {
    async fn transform_request(
        &self,
        body: &[u8],
        _headers: &HeaderMap,
    ) -> Result<LlmRequest, InboundError> {
        let request: OpenAiChatRequest = serde_json::from_slice(body)
            .map_err(|e| InboundError::ParseError(format!("解析 OpenAI Chat 请求失败: {}", e)))?;

        let messages = request
            .messages
            .into_iter()
            .map(|m| {
                let role = match m.role.as_str() {
                    "system" => Role::System,
                    "user" => Role::User,
                    "assistant" => Role::Assistant,
                    "tool" => Role::Tool,
                    "developer" => Role::Developer,
                    _ => Role::User,
                };

                let content = m.content.map(|c| {
                    if let Some(s) = c.as_str() {
                        Content::Text(s.to_string())
                    } else {
                        Content::Parts(vec![ContentPart::Text {
                            text: serde_json::to_string(&c).unwrap_or_default(),
                            cache_control: None,
                        }])
                    }
                });

                let tool_calls = m.tool_calls.map(|tc| {
                    tc.into_iter()
                        .map(|t| ToolCall {
                            id: t.id,
                            call_type: t.call_type,
                            function: FunctionCall {
                                name: t.function.name,
                                arguments: t.function.arguments,
                            },
                        })
                        .collect()
                });

                Message {
                    role,
                    content,
                    name: m.name,
                    tool_calls,
                    tool_call_id: m.tool_call_id,
                    reasoning_content: None,
                    cache_control: None,
                }
            })
            .collect();

        let tools = request.tools.map(|t| {
            t.into_iter()
                .map(|tool| Tool {
                    tool_type: tool.tool_type,
                    function: FunctionDefinition {
                        name: tool.function.name,
                        description: tool.function.description,
                        parameters: tool.function.parameters,
                    },
                })
                .collect()
        });

        let tool_choice = request.tool_choice.and_then(|tc| {
            if tc == "none" {
                Some(ToolChoice::None)
            } else if tc == "auto" {
                Some(ToolChoice::Auto)
            } else if tc == "required" {
                Some(ToolChoice::Required)
            } else {
                tc.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .map(|name| ToolChoice::Function {
                        function: FunctionName {
                            name: name.to_string(),
                        },
                    })
            }
        });

        Ok(LlmRequest {
            model: request.model,
            messages,
            temperature: request.temperature,
            top_p: request.top_p,
            max_tokens: request.max_tokens,
            max_completion_tokens: request.max_completion_tokens,
            stream: request.stream,
            tools,
            tool_choice,
            stop: request.stop,
            reasoning_effort: None,
            extra: std::collections::HashMap::new(),
        })
    }

    fn transform_response(&self, response: &LlmResponse) -> Result<Vec<u8>, InboundError> {
        serde_json::to_vec(response)
            .map_err(|e| InboundError::TransformError(format!("序列化响应失败: {}", e)))
    }

    fn transform_stream_event(&self, event: &LlmStreamResponse) -> Result<Vec<u8>, InboundError> {
        let data = serde_json::to_string(event)
            .map_err(|e| InboundError::TransformError(format!("序列化流式事件失败: {}", e)))?;
        Ok(format!("data: {}\n\n", data).into_bytes())
    }

    fn create_stream_converter(&self) -> Box<dyn StreamConverter> {
        Box::new(SimpleStreamConverter::new(|event: &LlmStreamResponse| {
            let data = serde_json::to_string(event)
                .map_err(|e| InboundError::TransformError(format!("序列化流式事件失败: {}", e)))?;
            Ok(format!("data: {}\n\n", data).into_bytes())
        }))
    }
}
