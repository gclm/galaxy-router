use async_trait::async_trait;
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};

use super::inbound::{Inbound, InboundError};
use super::model::*;
use super::outbound::{Outbound, OutboundError};

/// OpenAI Chat Completions 入站转换器
pub struct OpenAiChatInbound;

/// OpenAI Chat Completions 出站转换器
pub struct OpenAiChatOutbound;

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
struct OpenAiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

/// OpenAI 工具调用
#[derive(Debug, Deserialize, Serialize)]
struct OpenAiToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: OpenAiFunctionCall,
}

/// OpenAI 函数调用
#[derive(Debug, Deserialize, Serialize)]
struct OpenAiFunctionCall {
    name: String,
    arguments: String,
}

/// OpenAI 工具
#[derive(Debug, Deserialize, Serialize)]
struct OpenAiTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenAiFunctionDefinition,
}

/// OpenAI 函数定义
#[derive(Debug, Deserialize, Serialize)]
struct OpenAiFunctionDefinition {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parameters: Option<serde_json::Value>,
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
}

#[async_trait]
impl Outbound for OpenAiChatOutbound {
    fn transform_request(&self, request: &LlmRequest) -> Result<Vec<u8>, OutboundError> {
        let messages: Vec<OpenAiMessage> = request
            .messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "tool",
                    Role::Developer => "developer",
                };

                let content = m.content.as_ref().map(|c| match c {
                    Content::Text(s) => serde_json::Value::String(s.clone()),
                    Content::Parts(parts) => {
                        serde_json::to_value(parts).unwrap_or(serde_json::Value::Null)
                    }
                });

                let tool_calls = m.tool_calls.as_ref().map(|tc| {
                    tc.iter()
                        .map(|t| OpenAiToolCall {
                            id: t.id.clone(),
                            call_type: t.call_type.clone(),
                            function: OpenAiFunctionCall {
                                name: t.function.name.clone(),
                                arguments: t.function.arguments.clone(),
                            },
                        })
                        .collect()
                });

                OpenAiMessage {
                    role: role.to_string(),
                    content,
                    name: m.name.clone(),
                    tool_calls,
                    tool_call_id: m.tool_call_id.clone(),
                }
            })
            .collect();

        let tools: Option<Vec<OpenAiTool>> = request.tools.as_ref().map(|t| {
            t.iter()
                .map(|tool| OpenAiTool {
                    tool_type: tool.tool_type.clone(),
                    function: OpenAiFunctionDefinition {
                        name: tool.function.name.clone(),
                        description: tool.function.description.clone(),
                        parameters: tool.function.parameters.clone(),
                    },
                })
                .collect()
        });

        let tool_choice = request.tool_choice.as_ref().map(|tc| match tc {
            ToolChoice::None => serde_json::Value::String("none".to_string()),
            ToolChoice::Auto => serde_json::Value::String("auto".to_string()),
            ToolChoice::Required => serde_json::Value::String("required".to_string()),
            ToolChoice::Function { function } => {
                serde_json::json!({ "type": "function", "function": { "name": function.name } })
            }
        });

        let body = serde_json::json!({
            "model": request.model,
            "messages": messages,
            "temperature": request.temperature,
            "top_p": request.top_p,
            "max_tokens": request.max_tokens,
            "max_completion_tokens": request.max_completion_tokens,
            "stream": request.stream,
            "tools": tools,
            "tool_choice": tool_choice,
            "stop": request.stop,
        });

        serde_json::to_vec(&body)
            .map_err(|e| OutboundError::TransformError(format!("序列化请求失败: {}", e)))
    }

    async fn transform_response(
        &self,
        body: &[u8],
        _status: u16,
    ) -> Result<LlmResponse, OutboundError> {
        serde_json::from_slice(body)
            .map_err(|e| OutboundError::ParseError(format!("解析 OpenAI Chat 响应失败: {}", e)))
    }

    fn transform_stream_event(
        &self,
        event: &[u8],
    ) -> Result<Option<LlmStreamResponse>, OutboundError> {
        let text = String::from_utf8_lossy(event);
        let text = text.trim();

        if text.is_empty() || text == "data: [DONE]" {
            return Ok(None);
        }

        let data = text.strip_prefix("data: ").unwrap_or(text);

        serde_json::from_str(data)
            .map(Some)
            .map_err(|e| OutboundError::ParseError(format!("解析 OpenAI Chat 流式事件失败: {}", e)))
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
