//! 思维链处理（thinking）响应插件。
//!
//! thinking-1：落库承接（累积 reasoning_content 喂 `resp_json["reasoning"]`）。
//! thinking-2：内容分离——识别正文 content 混入的 `<think>...</think>`，跨 chunk 状态机
//! 剥离归入 `reasoning_content`（转发侧）+ reasoning（落库）。覆盖 MiniMax/QwQ/DeepSeek 等
//! 把思维链当纯文本返回的上游。转发侧原生字段归一（reasoning_content↔thinking_delta）由
//! 协议转换器承担，本插件只做"标签剥离"这一空白点。
//!
//! 默认启用（`plugin.thinking_fix`，migration 17 默认 true）。

use async_trait::async_trait;
use serde_json::Value;

use super::{PluginContext, ResponsePlugin, StreamResponseProcessor, StreamResponsePlugin};
use crate::llm::protocol::model::{Content, LlmStreamResponse};

const OPEN_TAG: &str = "<think>";
const CLOSE_TAG: &str = "</think>";

/// 思维链响应插件（非流式 + 流式）。matches 所有 endpoint。
pub struct ThinkingFix;

#[async_trait]
impl ResponsePlugin for ThinkingFix {
    fn id(&self) -> &'static str {
        "thinking_fix"
    }
    fn matches(&self, _ctx: &PluginContext) -> bool {
        true
    }
    fn default_enabled(&self) -> bool {
        true
    }
    /// 非流式：一次性剥离完整 body 的 content 中 `<think>`（OpenAI Chat / Anthropic / Responses）。
    async fn rewrite_response(&self, mut body: Value, _ctx: &PluginContext) -> Value {
        strip_think_openai_chat(&mut body);
        strip_think_anthropic(&mut body);
        strip_think_responses(&mut body);
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

/// 流式思维链 processor：跨 chunk 状态机剥离 `<think>...</think>`。
#[derive(Default)]
struct ThinkingProcessor {
    /// 累积的 reasoning（原生 reasoning_content + 剥离的 think），供落库
    reasoning: String,
    /// 是否在 `<think>` 块内
    in_think: bool,
    /// chunk 末尾未消歧的标签前缀缓冲（如 "<thi" / "</th"），跨 chunk 拼接
    pending: String,
}

impl ThinkingProcessor {
    /// 处理一段 content 文本，剥离 `<think>...</think>` 归 reasoning。
    /// 返回 (正文, 本段剥离的 reasoning)。跨 chunk 状态由 in_think / pending 维护。
    fn process_chunk(&mut self, input: &str) -> (String, String) {
        let mut buf = std::mem::take(&mut self.pending);
        buf.push_str(input);

        let mut content = String::new();
        let mut reasoning = String::new();

        loop {
            let tag = if self.in_think { CLOSE_TAG } else { OPEN_TAG };
            match buf.find(tag) {
                Some(pos) => {
                    if self.in_think {
                        reasoning.push_str(&buf[..pos]);
                    } else {
                        content.push_str(&buf[..pos]);
                    }
                    buf = buf[pos + tag.len()..].to_string();
                    self.in_think = !self.in_think;
                }
                None => {
                    // 无完整 tag：末尾可能是 tag 前缀（跨 chunk），保留到 pending
                    let keep = retain_pending_prefix(&buf, tag);
                    let split = buf.len() - keep;
                    if self.in_think {
                        reasoning.push_str(&buf[..split]);
                    } else {
                        content.push_str(&buf[..split]);
                    }
                    self.pending = buf[split..].to_string();
                    break;
                }
            }
        }

        self.reasoning.push_str(&reasoning);
        (content, reasoning)
    }
}

impl StreamResponseProcessor for ThinkingProcessor {
    fn observe(&mut self, event: &mut LlmStreamResponse) {
        let Some(choice) = event.choices.get_mut(0) else {
            return;
        };
        let delta = &mut choice.delta;

        // 累积原生 reasoning_content（落库，承接 thinking-1）
        if let Some(r) = &delta.reasoning_content {
            self.reasoning.push_str(r);
        }

        // 改写 content：剥离 `<think>...</think>`
        let Some(content) = delta.content.take() else {
            return;
        };
        let Content::Text(text) = content else {
            delta.content = Some(content);
            return;
        };
        let (new_text, stripped) = self.process_chunk(&text);
        delta.content = if new_text.is_empty() {
            None
        } else {
            Some(Content::Text(new_text))
        };
        // 剥离的 reasoning 写入 delta.reasoning_content（转发侧，让 converter 转 thinking_delta 等）
        if !stripped.is_empty() {
            match &mut delta.reasoning_content {
                Some(r) => r.push_str(&stripped),
                None => delta.reasoning_content = Some(stripped),
            }
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

/// 返回 `buf` 末尾最长的、是 `tag` 前缀的字节数（用于跨 chunk 保留未消歧标签前缀）。
/// tag 是 ASCII（`<think>` / `</think>`），匹配字节必 ASCII，不会切断多字节字符。
fn retain_pending_prefix(buf: &str, tag: &str) -> usize {
    let max = buf.len().min(tag.len() - 1);
    (1..=max).rev().find(|&len| buf.ends_with(&tag[..len])).unwrap_or(0)
}

/// 一次性剥离完整文本中的 `<think>...</think>`：返回 (正文, 拼接的 reasoning)。
/// 未闭合的 `<think>`：其后全部归 reasoning。
fn strip_think(text: &str) -> (String, String) {
    let mut clean = String::new();
    let mut reasoning = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(OPEN_TAG) {
        clean.push_str(&rest[..start]);
        let after = &rest[start + OPEN_TAG.len()..];
        match after.find(CLOSE_TAG) {
            Some(end) => {
                reasoning.push_str(&after[..end]);
                rest = &after[end + CLOSE_TAG.len()..];
            }
            None => {
                reasoning.push_str(after);
                return (clean, reasoning);
            }
        }
    }
    clean.push_str(rest);
    (clean, reasoning)
}

/// OpenAI Chat: `choices[].message.content`（string）剥离，reasoning 归 `message.reasoning_content`
fn strip_think_openai_chat(body: &mut Value) {
    let Some(choices) = body.get_mut("choices").and_then(|c| c.as_array_mut()) else {
        return;
    };
    for choice in choices {
        let Some(content) = choice
            .get_mut("message")
            .and_then(|m| m.get_mut("content"))
            .and_then(|c| c.as_str().map(String::from))
        else {
            continue;
        };
        let (clean, reasoning) = strip_think(&content);
        let msg = choice.get_mut("message").expect("已在上文取到");
        if !reasoning.is_empty() {
            match msg.get_mut("reasoning_content") {
                Some(Value::String(r)) => r.push_str(&reasoning),
                _ => {
                    msg["reasoning_content"] = Value::String(reasoning);
                }
            }
        }
        msg["content"] = Value::String(clean);
    }
}

/// Anthropic: `content[].text` 剥离，reasoning 归新的 thinking block（插入首位）
fn strip_think_anthropic(body: &mut Value) {
    let Some(blocks) = body.get_mut("content").and_then(|c| c.as_array_mut()) else {
        return;
    };
    let mut all_reasoning = String::new();
    for block in blocks.iter_mut() {
        if block.get("type").and_then(|t| t.as_str()) == Some("text")
            && let Some(text) = block.get("text").and_then(|t| t.as_str()).map(String::from)
        {
            let (clean, reasoning) = strip_think(&text);
            if !reasoning.is_empty() {
                all_reasoning.push_str(&reasoning);
            }
            block["text"] = Value::String(clean);
        }
    }
    if !all_reasoning.is_empty() {
        blocks.insert(0, serde_json::json!({"type": "thinking", "thinking": all_reasoning}));
    }
}

/// OpenAI Responses: `output[].content[].text` 剥离
fn strip_think_responses(body: &mut Value) {
    let Some(output) = body.get_mut("output").and_then(|o| o.as_array_mut()) else {
        return;
    };
    for item in output.iter_mut() {
        let Some(contents) = item.get_mut("content").and_then(|c| c.as_array_mut()) else {
            continue;
        };
        for part in contents.iter_mut() {
            if let Some(text) = part.get("text").and_then(|t| t.as_str()).map(String::from) {
                let (clean, _) = strip_think(&text);
                part["text"] = Value::String(clean);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::channel::EndpointType;
    use crate::llm::protocol::model::{Message, Role, StreamChoice};
    use serde_json::json;

    fn ctx() -> PluginContext {
        PluginContext {
            upstream_endpoint: EndpointType::OpenAiChat,
            channel_id: "c".into(),
            host_key: "h".into(),
            client_name: None,
        }
    }

    fn stream_with_content(text: &str) -> LlmStreamResponse {
        LlmStreamResponse {
            id: "1".into(),
            object: "chat.completion.chunk".into(),
            created: 0,
            model: "m".into(),
            choices: vec![StreamChoice {
                index: 0,
                delta: Message {
                    role: Role::User,
                    content: Some(Content::Text(text.into())),
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
        }
    }

    /// 提取 delta.content 的文本（测试辅助，避免 Content: PartialEq 依赖）
    fn content_text(ev: &LlmStreamResponse) -> String {
        match &ev.choices[0].delta.content {
            Some(Content::Text(t)) => t.clone(),
            _ => String::new(),
        }
    }

    // --- retain_pending_prefix ---

    #[test]
    fn retain_prefix_matches_partial_open_tag() {
        assert_eq!(retain_pending_prefix("hello<thi", OPEN_TAG), 4); // "<thi"
        assert_eq!(retain_pending_prefix("a<", OPEN_TAG), 1);
    }

    #[test]
    fn retain_prefix_zero_when_no_match() {
        assert_eq!(retain_pending_prefix("hello world", OPEN_TAG), 0);
        assert_eq!(retain_pending_prefix("普通文本", OPEN_TAG), 0);
    }

    #[test]
    fn retain_prefix_respects_max_tag_len() {
        // 不应保留超过 tag.len()-1 的字节
        assert_eq!(retain_pending_prefix("<think><think", OPEN_TAG), OPEN_TAG.len() - 1);
    }

    // --- strip_think（非流式，完整文本）---

    #[test]
    fn strip_think_single_block() {
        let (clean, reasoning) = strip_think("before<think>secret</think>after");
        assert_eq!(clean, "beforeafter");
        assert_eq!(reasoning, "secret");
    }

    #[test]
    fn strip_think_multiple_blocks() {
        let (clean, reasoning) = strip_think("a<think>1</think>b<think>2</think>c");
        assert_eq!(clean, "abc");
        assert_eq!(reasoning, "12");
    }

    #[test]
    fn strip_think_unclosed_takes_rest_as_reasoning() {
        let (clean, reasoning) = strip_think("text<think>ongoing");
        assert_eq!(clean, "text");
        assert_eq!(reasoning, "ongoing");
    }

    #[test]
    fn strip_think_no_tags_unchanged() {
        let (clean, reasoning) = strip_think("plain text");
        assert_eq!(clean, "plain text");
        assert!(reasoning.is_empty());
    }

    // --- process_chunk（流式，跨 chunk）---

    #[test]
    fn process_chunk_complete_block_in_one_chunk() {
        let mut p = ThinkingProcessor::default();
        let (content, reasoning) = p.process_chunk("hi<think>sec</think>!");
        assert_eq!(content, "hi!");
        assert_eq!(reasoning, "sec");
    }

    #[test]
    fn process_chunk_tag_split_across_chunks() {
        let mut p = ThinkingProcessor::default();
        // "<think>" 拆成 "<thi" + "nk>"
        let (c1, r1) = p.process_chunk("A<thi");
        assert_eq!(c1, "A");
        assert!(r1.is_empty());
        let (c2, r2) = p.process_chunk("nk>secret");
        assert!(c2.is_empty());
        assert_eq!(r2, "secret");
        // 闭合标签完整到达
        let (c3, _) = p.process_chunk("</think>B");
        assert_eq!(c3, "B");
    }

    #[test]
    fn process_chunk_close_tag_split_across_chunks() {
        let mut p = ThinkingProcessor::default();
        p.process_chunk("<think>sec"); // 进入 think，"sec" 归 reasoning
        let pending_before = p.pending.clone();
        // 闭合标签拆分："</thi" + "nk>"
        let (c, _) = p.process_chunk("</thi");
        assert!(c.is_empty());
        let (c2, _) = p.process_chunk("nk>tail");
        assert_eq!(c2, "tail");
        // 验证 pending 不残留（最后无歧义）
        assert!(pending_before.is_empty() || !p.pending.contains("nk>"));
    }

    #[test]
    fn process_chunk_unclosed_accumulates_until_end() {
        let mut p = ThinkingProcessor::default();
        let (c, r) = p.process_chunk("text<think>ongoing without close");
        assert_eq!(c, "text");
        assert_eq!(r, "ongoing without close");
    }

    #[test]
    fn process_chunk_preserves_multibyte_around_tag() {
        let mut p = ThinkingProcessor::default();
        let (c, _) = p.process_chunk("你好<think>x</think>世界");
        assert_eq!(c, "你好世界");
    }

    // --- observe（改写 LlmStreamResponse）---

    #[test]
    fn observe_strips_think_and_routes_to_reasoning_content() {
        let mut p = ThinkingProcessor::default();
        let mut ev = stream_with_content("hello<think>plan</think>world");
        p.observe(&mut ev);
        assert_eq!(content_text(&ev), "helloworld");
        assert_eq!(ev.choices[0].delta.reasoning_content.as_deref(), Some("plan"));
    }

    #[test]
    fn observe_splits_tag_across_events() {
        let mut p = ThinkingProcessor::default();
        let mut e1 = stream_with_content("A<thi");
        p.observe(&mut e1);
        assert_eq!(content_text(&e1), "A");
        assert!(e1.choices[0].delta.reasoning_content.is_none());

        let mut e2 = stream_with_content("nk>secret</think>B");
        p.observe(&mut e2);
        assert_eq!(content_text(&e2), "B");
        assert_eq!(e2.choices[0].delta.reasoning_content.as_deref(), Some("secret"));
    }

    #[test]
    fn observe_finish_reasoning_carries_stripped_content() {
        let mut p = ThinkingProcessor::default();
        let mut ev = stream_with_content("x<think>hidden</think>y");
        p.observe(&mut ev);
        assert_eq!(p.finish_reasoning().as_deref(), Some("hidden"));
    }

    // --- rewrite_response（非流式 body）---

    #[tokio::test]
    async fn rewrite_response_strips_openai_chat() {
        let body = json!({"choices":[{"message":{"content":"hi<think>secret</think>bye"}}]});
        let out = ThinkingFix.rewrite_response(body, &ctx()).await;
        assert_eq!(out["choices"][0]["message"]["content"], "hibye");
        assert_eq!(out["choices"][0]["message"]["reasoning_content"], "secret");
    }

    #[tokio::test]
    async fn rewrite_response_strips_anthropic_into_thinking_block() {
        let body = json!({"content":[{"type":"text","text":"a<think>ra</think>b"}]});
        let out = ThinkingFix.rewrite_response(body, &ctx()).await;
        assert_eq!(out["content"][1]["text"], "ab");
        assert_eq!(out["content"][0]["type"], "thinking");
        assert_eq!(out["content"][0]["thinking"], "ra");
    }

    #[tokio::test]
    async fn rewrite_response_no_think_unchanged() {
        let body = json!({"choices":[{"message":{"content":"plain"}}]});
        let out = ThinkingFix.rewrite_response(body.clone(), &ctx()).await;
        assert_eq!(out, body);
    }
}
