//! OpenAI Responses API 有状态流式转换器（从 openai_responses.rs 拆出）。
//!
//! 将统一 `LlmStreamResponse` 流转换为 Responses API SSE 事件序列，含完整生命周期：
//! response.created → output_item.added → delta → .done → response.completed。
//! 参考 AxonHub responsesInboundStream 实现。

use std::collections::HashMap;

use crate::llm::protocol::model::*;
use crate::llm::protocol::stream_converter::{StreamConvertError, StreamConverter};

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
            Self::sse_event(
                "response.created",
                serde_json::json!({
                    "type": "response.created",
                    "response": &response
                }),
            ),
            Self::sse_event(
                "response.in_progress",
                serde_json::json!({
                    "type": "response.in_progress",
                    "response": response
                }),
            ),
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
        events.push(Self::sse_event(
            "response.output_item.added",
            serde_json::json!({
                "type": "response.output_item.added",
                "output_index": self.output_index,
                "item": {
                    "id": item_id,
                    "type": "reasoning",
                    "status": "in_progress",
                    "summary": []
                }
            }),
        ));

        events
    }

    /// 处理 reasoning_content delta
    fn handle_reasoning_content(&mut self, reasoning: &str) -> Vec<Vec<u8>> {
        let mut events = self.ensure_reasoning_item_started();

        // 启动 reasoning summary part
        if !self.has_reasoning_summary_part {
            self.has_reasoning_summary_part = true;
            events.push(Self::sse_event(
                "response.reasoning_summary_part.added",
                serde_json::json!({
                    "type": "response.reasoning_summary_part.added",
                    "output_index": self.output_index,
                    "summary_index": 0,
                    "part": { "type": "summary_text" }
                }),
            ));
        }

        // 累积 reasoning
        self.accumulated_reasoning.push_str(reasoning);

        // 发送 delta
        events.push(Self::sse_event(
            "response.reasoning.delta",
            serde_json::json!({
                "type": "response.reasoning.delta",
                "output_index": self.output_index,
                "delta": reasoning
            }),
        ));

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
            events.push(Self::sse_event(
                "response.reasoning_summary_text.done",
                serde_json::json!({
                    "type": "response.reasoning_summary_text.done",
                    "output_index": self.output_index,
                    "summary_index": 0,
                    "text": full_reasoning
                }),
            ));
            events.push(Self::sse_event(
                "response.reasoning_summary_part.done",
                serde_json::json!({
                    "type": "response.reasoning_summary_part.done",
                    "output_index": self.output_index,
                    "summary_index": 0,
                    "part": { "type": "summary_text", "text": full_reasoning }
                }),
            ));
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

        events.push(Self::sse_event(
            "response.output_item.done",
            serde_json::json!({
                "type": "response.output_item.done",
                "output_index": self.output_index,
                "item": {
                    "type": "reasoning",
                    "summary": summary
                }
            }),
        ));

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
            events.push(Self::sse_event(
                "response.output_item.added",
                serde_json::json!({
                    "type": "response.output_item.added",
                    "output_index": self.output_index,
                    "item": {
                        "id": item_id,
                        "type": "message",
                        "status": "in_progress",
                        "role": "assistant",
                        "content": []
                    }
                }),
            ));
        }

        // 启动 content part
        if !self.has_content_part_started {
            self.has_content_part_started = true;
            events.push(Self::sse_event(
                "response.content_part.added",
                serde_json::json!({
                    "type": "response.content_part.added",
                    "output_index": self.output_index,
                    "content_index": self.content_index,
                    "part": { "type": "output_text", "text": "" }
                }),
            ));
        }

        // 累积文本
        self.accumulated_text.push_str(text);

        // 发送 delta
        events.push(Self::sse_event(
            "response.output_text.delta",
            serde_json::json!({
                "type": "response.output_text.delta",
                "output_index": self.output_index,
                "content_index": self.content_index,
                "delta": text
            }),
        ));

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

                events.push(Self::sse_event(
                    "response.output_item.added",
                    serde_json::json!({
                        "type": "response.output_item.added",
                        "output_index": self.output_index,
                        "item": {
                            "type": "function_call",
                            "id": tc.id,
                            "status": "in_progress",
                            "call_id": tc.id,
                            "name": tc.function.name
                        }
                    }),
                ));

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
            Self::sse_event(
                "response.output_text.done",
                serde_json::json!({
                    "type": "response.output_text.done",
                    "output_index": self.output_index,
                    "content_index": self.content_index,
                    "text": full_text
                }),
            ),
            Self::sse_event(
                "response.content_part.done",
                serde_json::json!({
                    "type": "response.content_part.done",
                    "output_index": self.output_index,
                    "content_index": self.content_index,
                    "part": { "type": "output_text", "text": full_text }
                }),
            ),
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

        events.push(Self::sse_event(
            "response.output_item.done",
            serde_json::json!({
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
            }),
        ));

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
            events.push(Self::sse_event(
                "response.output_item.done",
                serde_json::json!({
                    "type": "response.output_item.done",
                    "output_index": state.output_index,
                    "item": {
                        "type": "function_call",
                        "status": "completed",
                        "call_id": state.id,
                        "name": state.name,
                        "arguments": state.arguments
                    }
                }),
            ));
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

            events.push(Self::sse_event(
                "response.completed",
                serde_json::json!({
                    "type": "response.completed",
                    "response": response_obj
                }),
            ));
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

            events.push(Self::sse_event(
                "response.completed",
                serde_json::json!({
                    "type": "response.completed",
                    "response": response_obj
                }),
            ));
        }

        Ok(events)
    }
}
