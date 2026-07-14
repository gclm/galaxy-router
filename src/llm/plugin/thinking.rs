//! 思维链处理（thinking）响应插件。
//!
//! thinking-1：落库承接——删 stream_executor 内联 reasoning 收集（原 445-775）后，由
//! `ThinkingProcessor` 在流式 hook 里累积 `reasoning_content`，流结束喂
//! `resp_json["reasoning"]` 落库（行为等价）。转发侧字段归一化由现有协议转换器承担，
//! 本插件不重复；非流式 `rewrite_response` 暂 no-op 占位。
//!
//! thinking-2（待续）：扩展 `ThinkingProcessor` 为跨 chunk 状态机，识别正文 content
//! 混入的 `<think>...</think>` 并剥离归入 reasoning（新能力）。
//!
//! 默认启用（`plugin.thinking_fix`，migration 17 默认 true）：承接的是 v1.0 已有能力。

use async_trait::async_trait;
use serde_json::Value;

use super::{PluginContext, ResponsePlugin, StreamResponseProcessor, StreamResponsePlugin};
use crate::llm::protocol::model::LlmStreamResponse;

/// 思维链响应插件（非流式 + 流式）。matches 所有 endpoint（reasoning 字段跨协议都需收集）。
pub struct ThinkingFix;

#[async_trait]
impl ResponsePlugin for ThinkingFix {
    fn id(&self) -> &'static str {
        "thinking_fix"
    }
    fn matches(&self, _ctx: &PluginContext) -> bool {
        true
    }
    /// 承接 v1.0 能力，默认启用（setting 缺失兜底）
    fn default_enabled(&self) -> bool {
        true
    }
    /// thinking-1：no-op 占位（转发侧归一由转换器承担）。thinking-2 填内容分离。
    async fn rewrite_response(&self, body: Value, _ctx: &PluginContext) -> Value {
        body
    }
}

impl StreamResponsePlugin for ThinkingFix {
    fn id(&self) -> &'static str {
        "thinking_fix"
    }
    fn matches(&self, _ctx: &PluginContext) -> bool {
        true
    }
    fn default_enabled(&self) -> bool {
        true
    }
    fn new_processor(&self) -> Box<dyn StreamResponseProcessor> {
        Box::new(ThinkingProcessor::default())
    }
}

/// 流式思维链 processor：累积 `reasoning_content` 供落库。
#[derive(Default)]
struct ThinkingProcessor {
    reasoning: String,
}

impl StreamResponseProcessor for ThinkingProcessor {
    fn observe(&mut self, event: &LlmStreamResponse) {
        if let Some(choice) = event.first_choice()
            && let Some(r) = &choice.delta.reasoning_content
        {
            self.reasoning.push_str(r);
        }
    }

    fn finish_reasoning(&mut self) -> Option<String> {
        if self.reasoning.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.reasoning))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::handlers::admin::channels::EndpointType;
    use crate::llm::protocol::model::{LlmStreamResponse, Message, Role, StreamChoice};
    use serde_json::json;

    fn ctx() -> PluginContext {
        PluginContext {
            upstream_endpoint: EndpointType::OpenAiChat,
            channel_id: "c".into(),
            host_key: "h".into(),
            client_name: None,
        }
    }

    fn stream_resp(reasoning: Option<&str>) -> LlmStreamResponse {
        LlmStreamResponse {
            id: "chatcmpl-1".into(),
            object: "chat.completion.chunk".into(),
            created: 0,
            model: "m".into(),
            choices: vec![StreamChoice {
                index: 0,
                delta: Message {
                    role: Role::User,
                    content: None,
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                    reasoning_content: reasoning.map(String::from),
                    cache_control: None,
                },
                finish_reason: None,
            }],
            usage: None,
            system_fingerprint: None,
        }
    }

    #[tokio::test]
    async fn rewrite_response_is_noop_in_thinking1() {
        let body = json!({"choices": [{"message": {"content": "hi"}}]});
        let out = ThinkingFix.rewrite_response(body.clone(), &ctx()).await;
        assert_eq!(out, body);
    }

    #[test]
    fn processor_accumulates_reasoning_across_events() {
        let mut p = ThinkingProcessor::default();
        p.observe(&stream_resp(Some("part1")));
        p.observe(&stream_resp(Some("part2")));
        assert_eq!(p.finish_reasoning().as_deref(), Some("part1part2"));
    }

    #[test]
    fn processor_finish_none_when_no_reasoning() {
        let mut p = ThinkingProcessor::default();
        p.observe(&stream_resp(None));
        assert!(p.finish_reasoning().is_none());
    }

    #[test]
    fn processor_finish_none_when_empty() {
        let mut p = ThinkingProcessor::default();
        assert!(p.finish_reasoning().is_none());
    }

    #[test]
    fn matches_all_endpoints() {
        // 两 trait 都 match 所有 endpoint
        assert!(<ThinkingFix as ResponsePlugin>::matches(&ThinkingFix, &ctx()));
        assert!(<ThinkingFix as StreamResponsePlugin>::matches(&ThinkingFix, &ctx()));
    }

    #[test]
    fn new_processor_returns_thinking_processor() {
        let mut p = ThinkingFix.new_processor();
        // 空 processor finish 返回 None
        assert!(p.finish_reasoning().is_none());
    }
}
