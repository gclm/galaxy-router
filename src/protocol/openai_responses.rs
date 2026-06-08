use async_trait::async_trait;
use axum::http::HeaderMap;
use serde::Deserialize;
use std::collections::HashMap;

use super::inbound::{Inbound, InboundError};
use super::model::*;
use super::outbound::{Outbound, OutboundError};
use super::stream_converter::{StreamConverter, StreamConvertError};

/// OpenAI Responses 入站转换器
pub struct OpenAiResponsesInbound;

/// OpenAI Responses 出站转换器
pub struct OpenAiResponsesOutbound;

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

    fn create_stream_converter(&self) -> Box<dyn StreamConverter> {
        Box::new(ResponsesStreamConverter::new())
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

    fn set_auth_header(&self, headers: &mut reqwest::header::HeaderMap, api_key: &str) {
        headers.insert(
            "Authorization",
            format!("Bearer {}", api_key)
                .parse()
                .expect("api_key validated at save"),
        );
    }
}

// ============================================================================
// ResponsesStreamConverter — 有状态的流式转换器
// 参考 AxonHub responsesInboundStream 实现
// ============================================================================

/// Tool call 追踪状态
#[derive(Default)]
struct ToolCallState {
    id: String,
    name: String,
    arguments: String,
    output_index: usize,
    item_started: bool,
}

/// Responses API 有状态流式转换器
///
/// 将统一的 `LlmStreamResponse` 流转换为 Responses API SSE 事件序列，
/// 包含完整生命周期管理：response.created → output_item.added → delta → .done → response.completed
pub struct ResponsesStreamConverter {
    // 生命周期标记
    has_response_created: bool,
    has_message_item_started: bool,
    has_reasoning_item_started: bool,
    has_reasoning_summary_part: bool,
    has_content_part_started: bool,
    has_finished: bool,
    response_completed: bool,

    // 元数据（从首个 chunk 提取，后续复用）
    response_id: String,
    model: String,
    created_at: i64,

    // 位置追踪
    output_index: usize,
    content_index: usize,

    // 内容累积（用于 .done 事件）
    accumulated_text: String,
    accumulated_reasoning: String,

    // Tool call 追踪
    tool_calls: HashMap<usize, ToolCallState>,

    // Usage
    usage: Option<Usage>,
}

impl Default for ResponsesStreamConverter {
    fn default() -> Self {
        Self::new()
    }
}

impl ResponsesStreamConverter {
    pub fn new() -> Self {
        Self {
            has_response_created: false,
            has_message_item_started: false,
            has_reasoning_item_started: false,
            has_reasoning_summary_part: false,
            has_content_part_started: false,
            has_finished: false,
            response_completed: false,
            response_id: String::new(),
            model: String::new(),
            created_at: 0,
            output_index: 0,
            content_index: 0,
            accumulated_text: String::new(),
            accumulated_reasoning: String::new(),
            tool_calls: HashMap::new(),
            usage: None,
        }
    }

    /// 格式化一个 SSE 事件
    fn sse_event(event_type: &str, data: serde_json::Value) -> Vec<u8> {
        format!("event: {}\ndata: {}\n\n", event_type, data).into_bytes()
    }

    /// 确保已发送 response.created + response.in_progress
    fn ensure_response_created(&mut self, event: &LlmStreamResponse) -> Vec<Vec<u8>> {
        if self.has_response_created {
            return vec![];
        }
        self.has_response_created = true;

        // 从首个 chunk 提取元数据
        if self.response_id.is_empty() && !event.id.is_empty() {
            self.response_id = event.id.clone();
        }
        if self.model.is_empty() && !event.model.is_empty() {
            self.model = event.model.clone();
        }
        if self.created_at == 0 && event.created != 0 {
            self.created_at = event.created;
        }
        if self.usage.is_none() && event.usage.is_some() {
            self.usage = event.usage.clone();
        }

        let response = serde_json::json!({
            "id": self.response_id,
            "object": "response",
            "status": "in_progress",
            "model": self.model,
            "created_at": self.created_at,
            "output": []
        });

        vec![
            Self::sse_event("response.created", serde_json::json!({
                "type": "response.created",
                "response": &response
            })),
            Self::sse_event("response.in_progress", serde_json::json!({
                "type": "response.in_progress",
                "response": response
            })),
        ]
    }

    /// 确保 reasoning output item 已启动
    fn ensure_reasoning_item_started(&mut self) -> Vec<Vec<u8>> {
        if self.has_reasoning_item_started {
            return vec![];
        }

        // 先关闭当前打开的 output item
        let mut events = self.close_current_output_item();
        self.has_reasoning_item_started = true;
        self.has_reasoning_summary_part = false;

        let item_id = format!("rs_{}", self.output_index);
        events.push(Self::sse_event("response.output_item.added", serde_json::json!({
            "type": "response.output_item.added",
            "output_index": self.output_index,
            "item": {
                "id": item_id,
                "type": "reasoning",
                "status": "in_progress",
                "summary": []
            }
        })));

        events
    }

    /// 处理 reasoning_content delta
    fn handle_reasoning_content(&mut self, reasoning: &str) -> Vec<Vec<u8>> {
        let mut events = self.ensure_reasoning_item_started();

        // 启动 reasoning summary part
        if !self.has_reasoning_summary_part {
            self.has_reasoning_summary_part = true;
            events.push(Self::sse_event("response.reasoning_summary_part.added", serde_json::json!({
                "type": "response.reasoning_summary_part.added",
                "output_index": self.output_index,
                "summary_index": 0,
                "part": { "type": "summary_text" }
            })));
        }

        // 累积 reasoning
        self.accumulated_reasoning.push_str(reasoning);

        // 发送 delta
        events.push(Self::sse_event("response.reasoning.delta", serde_json::json!({
            "type": "response.reasoning.delta",
            "output_index": self.output_index,
            "delta": reasoning
        })));

        events
    }

    /// 关闭 reasoning item
    fn close_reasoning_item(&mut self) -> Vec<Vec<u8>> {
        if !self.has_reasoning_item_started {
            return vec![];
        }
        self.has_reasoning_item_started = false;

        let full_reasoning = self.accumulated_reasoning.clone();
        let mut events = Vec::new();

        // 如果有 summary part，发送 .done 事件
        if self.has_reasoning_summary_part {
            events.push(Self::sse_event("response.reasoning_summary_text.done", serde_json::json!({
                "type": "response.reasoning_summary_text.done",
                "output_index": self.output_index,
                "summary_index": 0,
                "text": full_reasoning
            })));
            events.push(Self::sse_event("response.reasoning_summary_part.done", serde_json::json!({
                "type": "response.reasoning_summary_part.done",
                "output_index": self.output_index,
                "summary_index": 0,
                "part": { "type": "summary_text", "text": full_reasoning }
            })));
        }
        self.has_reasoning_summary_part = false;

        // output_item.done
        let summary: Vec<serde_json::Value> = if self.accumulated_reasoning.is_empty() {
            vec![]
        } else {
            vec![serde_json::json!({
                "type": "summary_text",
                "text": full_reasoning
            })]
        };

        events.push(Self::sse_event("response.output_item.done", serde_json::json!({
            "type": "response.output_item.done",
            "output_index": self.output_index,
            "item": {
                "type": "reasoning",
                "summary": summary
            }
        })));

        self.output_index += 1;
        self.accumulated_reasoning.clear();

        events
    }

    /// 处理 text content delta
    fn handle_text_content(&mut self, text: &str) -> Vec<Vec<u8>> {
        // 关闭 reasoning item（如有）
        let mut events = self.close_reasoning_item();

        // 启动 message output item
        if !self.has_message_item_started {
            self.has_message_item_started = true;

            let item_id = format!("msg_{}", self.output_index);
            events.push(Self::sse_event("response.output_item.added", serde_json::json!({
                "type": "response.output_item.added",
                "output_index": self.output_index,
                "item": {
                    "id": item_id,
                    "type": "message",
                    "status": "in_progress",
                    "role": "assistant",
                    "content": []
                }
            })));
        }

        // 启动 content part
        if !self.has_content_part_started {
            self.has_content_part_started = true;
            events.push(Self::sse_event("response.content_part.added", serde_json::json!({
                "type": "response.content_part.added",
                "output_index": self.output_index,
                "content_index": self.content_index,
                "part": { "type": "output_text", "text": "" }
            })));
        }

        // 累积文本
        self.accumulated_text.push_str(text);

        // 发送 delta
        events.push(Self::sse_event("response.output_text.delta", serde_json::json!({
            "type": "response.output_text.delta",
            "output_index": self.output_index,
            "content_index": self.content_index,
            "delta": text
        })));

        events
    }

    /// 处理 tool_calls delta
    fn handle_tool_calls(&mut self, tool_calls: &[ToolCall]) -> Vec<Vec<u8>> {
        // 关闭 message + reasoning item
        let mut events = self.close_message_item();
        events.extend(self.close_reasoning_item());

        for tc in tool_calls {
            // 用 id 做 key（streaming 中 index 可能不稳定）
            let idx = if tc.id.is_empty() {
                0
            } else {
                // 使用 tool_calls map 的当前大小作为 index
                self.tool_calls.len()
            };

            // 首次出现：发送 output_item.added
            let is_new = !self.tool_calls.contains_key(&idx);
            if is_new {
                // 关闭之前的 content part 和 output item
                events.extend(self.close_current_content_part());
                if self.tool_calls.values().any(|s| s.item_started) {
                    // 之前的 tool call 需要关闭
                }

                self.tool_calls.insert(
                    idx,
                    ToolCallState {
                        id: tc.id.clone(),
                        name: tc.function.name.clone(),
                        arguments: String::new(),
                        output_index: self.output_index,
                        item_started: true,
                    },
                );

                events.push(Self::sse_event("response.output_item.added", serde_json::json!({
                    "type": "response.output_item.added",
                    "output_index": self.output_index,
                    "item": {
                        "type": "function_call",
                        "id": tc.id,
                        "status": "in_progress",
                        "call_id": tc.id,
                        "name": tc.function.name
                    }
                })));

                self.output_index += 1;
            }

            // 累积 arguments
            if let Some(state) = self.tool_calls.get_mut(&idx) {
                state.arguments.push_str(&tc.function.arguments);

                // 发送 delta
                if !tc.function.arguments.is_empty() {
                    events.push(Self::sse_event(
                        "response.function_call_arguments.delta",
                        serde_json::json!({
                            "type": "response.function_call_arguments.delta",
                            "output_index": state.output_index,
                            "call_id": state.id,
                            "delta": tc.function.arguments
                        }),
                    ));
                }
            }
        }

        events
    }

    /// 关闭当前 content part
    fn close_current_content_part(&mut self) -> Vec<Vec<u8>> {
        if !self.has_content_part_started {
            return vec![];
        }
        self.has_content_part_started = false;

        let full_text = self.accumulated_text.clone();

        vec![
            Self::sse_event("response.output_text.done", serde_json::json!({
                "type": "response.output_text.done",
                "output_index": self.output_index,
                "content_index": self.content_index,
                "text": full_text
            })),
            Self::sse_event("response.content_part.done", serde_json::json!({
                "type": "response.content_part.done",
                "output_index": self.output_index,
                "content_index": self.content_index,
                "part": { "type": "output_text", "text": full_text }
            })),
        ]
    }

    /// 关闭 message item
    fn close_message_item(&mut self) -> Vec<Vec<u8>> {
        if !self.has_message_item_started {
            return vec![];
        }
        self.has_message_item_started = false;

        let mut events = self.close_current_content_part();
        let full_text = self.accumulated_text.clone();

        events.push(Self::sse_event("response.output_item.done", serde_json::json!({
            "type": "response.output_item.done",
            "output_index": self.output_index,
            "item": {
                "type": "message",
                "status": "completed",
                "role": "assistant",
                "content": [{
                    "type": "output_text",
                    "text": full_text,
                    "annotations": []
                }]
            }
        })));

        self.output_index += 1;
        self.content_index = 0;
        self.accumulated_text.clear();

        events
    }

    /// 关闭所有打开的 output item
    fn close_current_output_item(&mut self) -> Vec<Vec<u8>> {
        let mut events = self.close_message_item();
        events.extend(self.close_reasoning_item());

        // 关闭所有 tool call items
        for state in self.tool_calls.values_mut() {
            if !state.item_started {
                continue;
            }
            state.item_started = false;

            events.push(Self::sse_event(
                "response.function_call_arguments.done",
                serde_json::json!({
                    "type": "response.function_call_arguments.done",
                    "output_index": state.output_index,
                    "call_id": state.id,
                    "arguments": state.arguments
                }),
            ));
            events.push(Self::sse_event("response.output_item.done", serde_json::json!({
                "type": "response.output_item.done",
                "output_index": state.output_index,
                "item": {
                    "type": "function_call",
                    "status": "completed",
                    "call_id": state.id,
                    "name": state.name,
                    "arguments": state.arguments
                }
            })));
        }

        events
    }

    /// 处理 finish_reason
    fn handle_finish(&mut self, event: &LlmStreamResponse) -> Vec<Vec<u8>> {
        if self.has_finished {
            return vec![];
        }
        self.has_finished = true;

        // 关闭所有打开的 items
        let mut events = self.close_current_output_item();

        // 更新 usage
        if event.usage.is_some() {
            self.usage = event.usage.clone();
        }

        // 发送 response.completed
        if !self.response_completed {
            self.response_completed = true;

            let mut response_obj = serde_json::json!({
                "id": self.response_id,
                "object": "response",
                "status": "completed",
                "model": self.model,
                "created_at": self.created_at,
                "output": []
            });

            if let Some(usage) = &self.usage {
                response_obj["usage"] = serde_json::json!({
                    "input_tokens": usage.prompt_tokens,
                    "output_tokens": usage.completion_tokens,
                    "total_tokens": usage.total_tokens,
                });
            }

            events.push(Self::sse_event("response.completed", serde_json::json!({
                "type": "response.completed",
                "response": response_obj
            })));
        }

        events
    }
}

impl StreamConverter for ResponsesStreamConverter {
    fn convert(&mut self, event: &LlmStreamResponse) -> Result<Vec<Vec<u8>>, StreamConvertError> {
        let mut all_events = Vec::new();

        // 确保已发送 response.created
        all_events.extend(self.ensure_response_created(event));

        // 更新元数据
        if self.response_id.is_empty() && !event.id.is_empty() {
            self.response_id = event.id.clone();
        }
        if self.model.is_empty() && !event.model.is_empty() {
            self.model = event.model.clone();
        }
        if self.created_at == 0 && event.created != 0 {
            self.created_at = event.created;
        }
        if event.usage.is_some() {
            self.usage = event.usage.clone();
        }

        if let Some(choice) = event.first_choice() {
            // 处理 reasoning_content
            if let Some(reasoning) = &choice.delta.reasoning_content
                && !reasoning.is_empty()
            {
                all_events.extend(self.handle_reasoning_content(reasoning));
            }

            // 处理 text content
            if let Some(content) = &choice.delta.content
                && let Content::Text(text) = content
                && !text.is_empty()
            {
                all_events.extend(self.handle_text_content(text));
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

        // 如果有 usage 但还没有 finish，且已经 finished，发送 completed
        if event.usage.is_some() && self.has_finished && !self.response_completed {
            all_events.extend(self.handle_finish(event));
        }

        Ok(all_events)
    }

    fn finish(&mut self) -> Result<Vec<Vec<u8>>, StreamConvertError> {
        // 如果已经 completed 则无需额外事件
        if self.response_completed {
            return Ok(vec![]);
        }

        let mut events = Vec::new();

        // 如果有 finish 但还没发送 completed
        if self.has_finished && !self.response_completed {
            self.response_completed = true;

            let mut response_obj = serde_json::json!({
                "id": self.response_id,
                "object": "response",
                "status": "completed",
                "model": self.model,
                "created_at": self.created_at,
                "output": []
            });

            if let Some(usage) = &self.usage {
                response_obj["usage"] = serde_json::json!({
                    "input_tokens": usage.prompt_tokens,
                    "output_tokens": usage.completion_tokens,
                    "total_tokens": usage.total_tokens,
                });
            }

            events.push(Self::sse_event("response.completed", serde_json::json!({
                "type": "response.completed",
                "response": response_obj
            })));
        }

        Ok(events)
    }
}
