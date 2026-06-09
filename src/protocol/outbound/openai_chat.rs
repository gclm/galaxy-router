use async_trait::async_trait;

use super::{Outbound, OutboundError};
use crate::protocol::inbound::openai_chat::{
    OpenAiFunctionCall, OpenAiFunctionDefinition, OpenAiMessage, OpenAiTool, OpenAiToolCall,
};
use crate::protocol::model::*;

/// OpenAI Chat Completions 出站转换器
pub struct OpenAiChatOutbound;

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
