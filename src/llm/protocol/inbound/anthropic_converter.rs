//! Anthropic Messages API 有状态流式转换器（从 anthropic.rs 拆出）。
//!
//! 将统一 `LlmStreamResponse` 流转换为 Anthropic SSE 事件序列，含完整 content block 生命周期：
//! message_start → content_block_start → content_block_delta → content_block_stop → message_delta → message_stop。
//! 参考 AxonHub anthropicInboundStream 实现。

use std::collections::HashMap;

use crate::llm::protocol::model::*;
use crate::llm::protocol::stream_converter::{StreamConvertError, StreamConverter};

/// Tool call 追踪状态
#[allow(dead_code)]
struct AnthropicToolCallState {
    id: String,
    name: String,
    arguments: String,
}

/// Anthropic Messages 有状态流式转换器
///
/// 将统一的 `LlmStreamResponse` 流转换为 Anthropic SSE 事件序列，
/// 包含完整 content block 生命周期：message_start → content_block_start →
/// content_block_delta → content_block_stop → message_delta → message_stop
pub struct AnthropicStreamConverter {
    // 生命周期标记
    has_started: bool,
    has_thinking_content_started: bool,
    has_text_content_started: bool,
    has_tool_content_started: bool,
    has_finished: bool,
    message_stopped: bool,

    // 元数据（从首个 chunk 提取，后续复用）
    message_id: String,
    model: String,

    // 内容块索引
    content_index: usize,

    // 当前 tool call 追踪
    current_tool_index: usize,
    tool_calls: HashMap<usize, AnthropicToolCallState>,

    // Usage（Anthropic 分 message_start 和 message_delta 两次发）
    input_usage: Option<Usage>,

    // 去重
    last_event_type: String,
}

impl Default for AnthropicStreamConverter {
    fn default() -> Self {
        Self::new()
    }
}

impl AnthropicStreamConverter {
    pub fn new() -> Self {
        Self {
            has_started: false,
            has_thinking_content_started: false,
            has_text_content_started: false,
            has_tool_content_started: false,
            has_finished: false,
            message_stopped: false,
            message_id: String::new(),
            model: String::new(),
            content_index: 0,
            current_tool_index: 0,
            tool_calls: HashMap::new(),
            input_usage: None,
            last_event_type: String::new(),
        }
    }

    /// 格式化一个 SSE 事件
    fn sse_event(event_type: &str, data: serde_json::Value) -> Vec<u8> {
        format!("event: {}\ndata: {}\n\n", event_type, data).into_bytes()
    }

    /// 去重检查：防止重复 content_block_stop
    fn should_emit(&mut self, event_type: &str) -> bool {
        if self.last_event_type == "content_block_stop" && event_type == "content_block_stop" {
            return false;
        }
        self.last_event_type = event_type.to_string();
        true
    }

    /// 发送 message_start（仅首次）
    fn emit_message_start(&mut self, event: &LlmStreamResponse) -> Vec<Vec<u8>> {
        if self.has_started {
            return vec![];
        }
        self.has_started = true;

        // 从首个 chunk 提取元数据
        if self.message_id.is_empty() && !event.id.is_empty() {
            self.message_id = event.id.clone();
        }
        if self.model.is_empty() && !event.model.is_empty() {
            self.model = event.model.clone();
        }
        if event.usage.is_some() {
            self.input_usage = event.usage.clone();
        }

        let usage = serde_json::json!({
            "input_tokens": self.input_usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(1),
            "output_tokens": self.input_usage.as_ref().map(|u| u.completion_tokens).unwrap_or(1)
        });

        vec![Self::sse_event(
            "message_start",
            serde_json::json!({
                "type": "message_start",
                "message": {
                    "id": self.message_id,
                    "type": "message",
                    "role": "assistant",
                    "content": [],
                    "model": self.model,
                    "stop_reason": null,
                    "usage": usage
                }
            }),
        )]
    }

    /// 关闭 thinking block
    fn close_thinking_block(&mut self) -> Vec<Vec<u8>> {
        if !self.has_thinking_content_started {
            return vec![];
        }
        self.has_thinking_content_started = false;

        let mut events = vec![];
        if self.should_emit("content_block_stop") {
            events.push(Self::sse_event(
                "content_block_stop",
                serde_json::json!({
                    "type": "content_block_stop",
                    "index": self.content_index
                }),
            ));
        }
        self.content_index += 1;
        events
    }

    /// 关闭 text block
    fn close_text_block(&mut self) -> Vec<Vec<u8>> {
        if !self.has_text_content_started {
            return vec![];
        }
        self.has_text_content_started = false;

        let mut events = vec![];
        if self.should_emit("content_block_stop") {
            events.push(Self::sse_event(
                "content_block_stop",
                serde_json::json!({
                    "type": "content_block_stop",
                    "index": self.content_index
                }),
            ));
        }
        self.content_index += 1;
        events
    }

    /// 关闭 tool block
    fn close_tool_block(&mut self) -> Vec<Vec<u8>> {
        if !self.has_tool_content_started {
            return vec![];
        }
        self.has_tool_content_started = false;

        let mut events = vec![];
        if self.should_emit("content_block_stop") {
            events.push(Self::sse_event(
                "content_block_stop",
                serde_json::json!({
                    "type": "content_block_stop",
                    "index": self.content_index
                }),
            ));
        }
        self.content_index += 1;
        events
    }

    /// 处理 reasoning_content（thinking block）
    fn handle_thinking(&mut self, thinking: &str) -> Vec<Vec<u8>> {
        let mut events = vec![];

        // 关闭其他打开的 block
        events.extend(self.close_tool_block());
        events.extend(self.close_text_block());

        // 启动 thinking block
        if !self.has_thinking_content_started {
            self.has_thinking_content_started = true;
            events.push(Self::sse_event(
                "content_block_start",
                serde_json::json!({
                    "type": "content_block_start",
                    "index": self.content_index,
                    "content_block": { "type": "thinking" }
                }),
            ));
        }

        // 发送 thinking delta
        events.push(Self::sse_event(
            "content_block_delta",
            serde_json::json!({
                "type": "content_block_delta",
                "index": self.content_index,
                "delta": { "type": "thinking_delta", "thinking": thinking }
            }),
        ));

        events
    }

    /// 处理 text content
    fn handle_text(&mut self, text: &str) -> Vec<Vec<u8>> {
        let mut events = vec![];

        // 关闭 thinking 和 tool block
        events.extend(self.close_thinking_block());
        events.extend(self.close_tool_block());

        // 启动 text block
        if !self.has_text_content_started {
            self.has_text_content_started = true;
            events.push(Self::sse_event(
                "content_block_start",
                serde_json::json!({
                    "type": "content_block_start",
                    "index": self.content_index,
                    "content_block": { "type": "text" }
                }),
            ));
        }

        // 发送 text delta
        events.push(Self::sse_event(
            "content_block_delta",
            serde_json::json!({
                "type": "content_block_delta",
                "index": self.content_index,
                "delta": { "type": "text_delta", "text": text }
            }),
        ));

        events
    }

    /// 处理 tool_calls
    fn handle_tool_calls(&mut self, tool_calls: &[StreamToolCallDelta]) -> Vec<Vec<u8>> {
        let mut events = vec![];

        // 关闭 thinking 和 text block
        events.extend(self.close_thinking_block());
        events.extend(self.close_text_block());

        for tc in tool_calls {
            // 用 index 追踪（streaming 中同一个 tool call 会多次出现）
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
            let idx = self.current_tool_index;

            let is_new = !self.tool_calls.contains_key(&idx);
            if is_new {
                // 关闭之前的 tool block
                if self.has_tool_content_started {
                    events.extend(self.close_tool_block());
                }
                self.has_tool_content_started = true;

                self.tool_calls.insert(
                    idx,
                    AnthropicToolCallState {
                        id: id.clone(),
                        name: name.clone(),
                        arguments: String::new(),
                    },
                );

                events.push(Self::sse_event(
                    "content_block_start",
                    serde_json::json!({
                        "type": "content_block_start",
                        "index": self.content_index,
                        "content_block": {
                            "type": "tool_use",
                            "id": id,
                            "name": name,
                            "input": {}
                        }
                    }),
                ));

                // 如果有初始 arguments，发送 delta
                if !arguments.is_empty() {
                    events.push(Self::sse_event("content_block_delta", serde_json::json!({
                        "type": "content_block_delta",
                        "index": self.content_index,
                        "delta": { "type": "input_json_delta", "partial_json": arguments }
                    })));
                    if let Some(state) = self.tool_calls.get_mut(&idx) {
                        state.arguments.push_str(&arguments);
                    }
                }

                self.current_tool_index += 1;
            } else if let Some(state) = self.tool_calls.get_mut(&idx) {
                // 追加 arguments
                state.arguments.push_str(&arguments);
                if !arguments.is_empty() {
                    events.push(Self::sse_event("content_block_delta", serde_json::json!({
                        "type": "content_block_delta",
                        "index": self.content_index,
                        "delta": { "type": "input_json_delta", "partial_json": arguments }
                    })));
                }
            }
        }

        events
    }

    /// 处理 finish_reason
    fn handle_finish(&mut self, event: &LlmStreamResponse) -> Vec<Vec<u8>> {
        if self.has_finished {
            return vec![];
        }
        self.has_finished = true;

        let mut events = vec![];

        // 关闭 thinking block
        events.extend(self.close_thinking_block());

        // 关闭当前 content block
        if self.has_tool_content_started {
            events.extend(self.close_tool_block());
        } else if self.has_text_content_started {
            events.extend(self.close_text_block());
        }

        // 映射 finish_reason
        let stop_reason = event
            .first_choice()
            .and_then(|c| c.finish_reason.as_ref())
            .map(|fr| match fr {
                FinishReason::Stop => "end_turn",
                FinishReason::Length => "max_tokens",
                FinishReason::ToolCalls => "tool_use",
                _ => "end_turn",
            })
            .unwrap_or("end_turn");

        // 发送 message_delta + message_stop
        if !self.message_stopped {
            self.message_stopped = true;

            let usage = if let Some(u) = &event.usage {
                serde_json::json!({
                    "output_tokens": u.completion_tokens
                })
            } else {
                serde_json::json!({ "output_tokens": 0 })
            };

            events.push(Self::sse_event(
                "message_delta",
                serde_json::json!({
                    "type": "message_delta",
                    "delta": {
                        "stop_reason": stop_reason
                    },
                    "usage": usage
                }),
            ));
            events.push(Self::sse_event(
                "message_stop",
                serde_json::json!({
                    "type": "message_stop"
                }),
            ));
        }

        events
    }
}

impl StreamConverter for AnthropicStreamConverter {
    fn convert(&mut self, event: &LlmStreamResponse) -> Result<Vec<Vec<u8>>, StreamConvertError> {
        let mut all_events = Vec::new();

        // 确保已发送 message_start
        all_events.extend(self.emit_message_start(event));

        // 更新元数据
        if self.message_id.is_empty() && !event.id.is_empty() {
            self.message_id = event.id.clone();
        }
        if self.model.is_empty() && !event.model.is_empty() {
            self.model = event.model.clone();
        }
        if event.usage.is_some() {
            self.input_usage = event.usage.clone();
        }

        if let Some(choice) = event.first_choice() {
            // 处理 reasoning_content（thinking block）
            if let Some(reasoning) = &choice.delta.reasoning_content
                && !reasoning.is_empty()
            {
                all_events.extend(self.handle_thinking(reasoning));
            }

            // 处理 text content
            if let Some(content) = &choice.delta.content
                && let Content::Text(text) = content
                && !text.is_empty()
            {
                all_events.extend(self.handle_text(text));
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

        // 如果有 usage 但还没 finished
        if event.usage.is_some() && self.has_finished && !self.message_stopped {
            all_events.extend(self.handle_finish(event));
        }

        Ok(all_events)
    }

    fn finish(&mut self) -> Result<Vec<Vec<u8>>, StreamConvertError> {
        if self.message_stopped {
            return Ok(vec![]);
        }
        Ok(vec![])
    }
}
