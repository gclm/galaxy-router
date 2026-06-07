//! 思维链规范化器
//!
//! 处理两类上游供应商的非标准行为：
//!
//! 1. **`<think/>` 标签嵌入 content**（如 MiniMax-M3）
//!    - 上游把 thinking 内容用 `<think>...</think>` 包裹后直接放进 `delta.content`
//!    - 标签可能跨多个 SSE chunk，需要状态机解析
//!    - 抽取后写入 `reasoning_content` 字段，清理后的 content 写回原字段
//!
//! 2. **Anthropic signature 位置不对**（如 GLM-5.1）
//!    - 上游把 `signature` 放在 `content_block_start` 事件体里
//!    - 标准协议要求发送独立的 `signature_delta` 事件
//!    - 修复方式：缓存 signature，在 `content_block_stop` 之前补发
//!
//! 设计原则：
//! - 当 `thinking_mode = NULL` 时，调用方不创建 normalizer，零开销
//! - 状态机逐 chunk 处理，不依赖全量缓冲
//! - 直通路径只在需要时解析 JSON，避免影响性能

use crate::api::handlers::admin::channels::EndpointType;
use crate::protocol::model::{Content, LlmStreamResponse};
use serde_json::Value;

/// 一次 feed 操作的输出
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ParserOutput {
    /// 正常正文内容（已剥离 `<think/>` 标签）
    pub content: String,
    /// 抽取出的思维链内容
    pub thinking: String,
}

/// 解析器内部状态
#[derive(Debug, Clone, PartialEq, Eq)]
enum ParseState {
    /// 正常正文
    Normal,
    /// 已读到 `<`，正在匹配 `think`
    /// n 表示已匹配的字符数（0..=5）
    OpenMatch(u8),
    /// 已匹配 `<think`，正在等待 `>`
    /// 期间允许出现空白字符；`pending` 累计已跳过的空白
    /// 若遇到非 `>` 字符 → 整个 `<think` + pending + 当前字符 都作为 content
    OpenClosing { pending: String },
    /// 在 `<think...>` 内部
    InsideThink,
    /// 在 `<think>` 内部读到 `<`，正在匹配 `/think`
    /// n 表示已匹配的字符数（0..=6）
    CloseMatch(u8),
    /// 已匹配 `</think`，正在等待 `>`
    /// 期间允许空白
    CloseClosing { pending: String },
}

const OPEN_TARGET: &str = "think";
const CLOSE_TARGET: &str = "/think";

/// `<think/>` 标签解析器
pub struct ThinkTagParser {
    state: ParseState,
    /// Normal 状态下积累的 content
    content_buf: String,
    /// InsideThink 状态下积累的 thinking
    thinking_buf: String,
}

impl Default for ThinkTagParser {
    fn default() -> Self {
        Self::new()
    }
}

impl ThinkTagParser {
    pub fn new() -> Self {
        Self {
            state: ParseState::Normal,
            content_buf: String::new(),
            thinking_buf: String::new(),
        }
    }

    /// 喂入一段文本，返回解析后产生的内容输出
    ///
    /// 设计：每次 feed 调用只返回**本调用产生的新增** content。
    /// 内部 content_buf 保留未产出的部分（用于后续拼接 thinking 闭合等场景）。
    /// thinking 在 `</think` 完整匹配时立即产出，不依赖 feed 边界。
    pub fn feed(&mut self, text: &str) -> ParserOutput {
        let mut out = ParserOutput::default();
        for ch in text.chars() {
            // 取出 state，处理后放回（这样可以处理含 String 的变体）
            let prev = std::mem::replace(&mut self.state, ParseState::Normal);
            let (next, _) = self.process(prev, ch, &mut out);
            self.state = next;
        }
        // 只取出本调用产生的新增 content（追加到 out.content，content_buf 保留未产出部分）
        out.content = std::mem::take(&mut self.content_buf);
        out
    }

    /// 流结束时调用
    pub fn flush(&mut self) -> ParserOutput {
        let mut out = ParserOutput::default();
        let prev = std::mem::replace(&mut self.state, ParseState::Normal);
        match prev {
            ParseState::Normal => {
                out.content = std::mem::take(&mut self.content_buf);
            }
            ParseState::OpenMatch(n) => {
                let s = rebuild_open(n);
                let mut combined = s;
                combined.push_str(&std::mem::take(&mut self.content_buf));
                out.content = combined;
            }
            ParseState::OpenClosing { pending } => {
                let mut s = String::from("<");
                s.push_str(OPEN_TARGET);
                s.push_str(&pending);
                s.push_str(&std::mem::take(&mut self.content_buf));
                out.content = s;
            }
            ParseState::InsideThink => {
                out.content = std::mem::take(&mut self.thinking_buf);
            }
            ParseState::CloseMatch(n) => {
                let s = rebuild_close(n);
                let mut combined = s;
                combined.push_str(&std::mem::take(&mut self.thinking_buf));
                out.thinking = combined;
            }
            ParseState::CloseClosing { pending } => {
                out.thinking = std::mem::take(&mut self.thinking_buf);
                let mut s = String::from("</");
                s.push_str(CLOSE_TARGET);
                s.push_str(&pending);
                out.content = s;
            }
        }
        out
    }

    /// 处理一个字符，返回 (新 state, 是否产出 thinking)
    fn process(
        &mut self,
        state: ParseState,
        ch: char,
        out: &mut ParserOutput,
    ) -> (ParseState, bool) {
        match state {
            ParseState::Normal => {
                if ch == '<' {
                    (ParseState::OpenMatch(0), false)
                } else {
                    self.content_buf.push(ch);
                    (ParseState::Normal, false)
                }
            }
            ParseState::OpenMatch(n) => {
                if (n as usize) < OPEN_TARGET.len() {
                    let expected = OPEN_TARGET.as_bytes()[n as usize] as char;
                    if ch == expected {
                        let new_n = n + 1;
                        if (new_n as usize) == OPEN_TARGET.len() {
                            (
                                ParseState::OpenClosing { pending: String::new() },
                                false,
                            )
                        } else {
                            (ParseState::OpenMatch(new_n), false)
                        }
                    } else {
                        // 匹配失败
                        self.content_buf.push_str(&rebuild_open(n));
                        if ch == '<' {
                            (ParseState::OpenMatch(0), false)
                        } else {
                            self.content_buf.push(ch);
                            (ParseState::Normal, false)
                        }
                    }
                } else {
                    (ParseState::InsideThink, false)
                }
            }
            ParseState::OpenClosing { mut pending } => {
                if ch == '>' {
                    (ParseState::InsideThink, false)
                } else if ch.is_whitespace() {
                    pending.push(ch);
                    (ParseState::OpenClosing { pending }, false)
                } else {
                    // 标签语法被破坏
                    let mut s = String::from("<");
                    s.push_str(OPEN_TARGET);
                    s.push_str(&pending);
                    s.push(ch);
                    self.content_buf.push_str(&s);
                    (ParseState::Normal, false)
                }
            }
            ParseState::InsideThink => {
                if ch == '<' {
                    (ParseState::CloseMatch(0), false)
                } else {
                    self.thinking_buf.push(ch);
                    (ParseState::InsideThink, false)
                }
            }
            ParseState::CloseMatch(n) => {
                if (n as usize) < CLOSE_TARGET.len() {
                    let expected = CLOSE_TARGET.as_bytes()[n as usize] as char;
                    if ch == expected {
                        let new_n = n + 1;
                        if (new_n as usize) == CLOSE_TARGET.len() {
                            // 完整匹配 `</think` → 产出 thinking
                            let thinking = std::mem::take(&mut self.thinking_buf);
                            append_thinking(&mut out.thinking, thinking);
                            // 进入 CloseClosing 等待 `>`
                            (
                                ParseState::CloseClosing { pending: String::new() },
                                true,
                            )
                        } else {
                            (ParseState::CloseMatch(new_n), false)
                        }
                    } else {
                        // 匹配失败
                        self.thinking_buf.push_str(&rebuild_close(n));
                        if ch == '<' {
                            (ParseState::CloseMatch(0), false)
                        } else {
                            self.thinking_buf.push(ch);
                            (ParseState::InsideThink, false)
                        }
                    }
                } else {
                    (
                        ParseState::CloseClosing { pending: String::new() },
                        false,
                    )
                }
            }
            ParseState::CloseClosing { mut pending } => {
                if ch == '>' {
                    // thinking 已在 `</think` 匹配时产出
                    (ParseState::Normal, true)
                } else if ch.is_whitespace() {
                    pending.push(ch);
                    (ParseState::CloseClosing { pending }, false)
                } else if ch == '<' {
                    // 异常：把 `</think[pending]` 作为 thinking 一部分
                    let mut s = String::from("</");
                    s.push_str(CLOSE_TARGET);
                    s.push_str(&pending);
                    self.thinking_buf.push_str(&s);
                    (ParseState::CloseMatch(0), false)
                } else {
                    // 异常：闭标签后非 `>` 非空白字符
                    let thinking = std::mem::take(&mut self.thinking_buf);
                    append_thinking(&mut out.thinking, thinking);
                    let mut s = String::from("</");
                    s.push_str(CLOSE_TARGET);
                    s.push_str(&pending);
                    s.push(ch);
                    self.content_buf.push_str(&s);
                    (ParseState::Normal, true)
                }
            }
        }
    }
}

fn rebuild_open(n: u8) -> String {
    let mut s = String::from("<");
    let take = (n as usize).min(OPEN_TARGET.len());
    s.push_str(&OPEN_TARGET[..take]);
    s
}

fn rebuild_close(n: u8) -> String {
    let mut s = String::from("</");
    let take = (n as usize).min(CLOSE_TARGET.len());
    s.push_str(&CLOSE_TARGET[..take]);
    s
}

fn append_thinking(target: &mut String, new: String) {
    if !new.is_empty() {
        if !target.is_empty() {
            target.push('\n');
        }
        target.push_str(&new);
    }
}

/// 转换路径规范化器：操作 LlmStreamResponse.delta.content
pub struct ThinkingTagExtractor {
    parser: ThinkTagParser,
}

impl Default for ThinkingTagExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl ThinkingTagExtractor {
    pub fn new() -> Self {
        Self {
            parser: ThinkTagParser::new(),
        }
    }

    /// 抽取 LlmStreamResponse 中的 `<think/>` 标签内容
    pub fn extract(&mut self, response: &mut LlmStreamResponse) -> bool {
        let mut modified = false;
        for choice in &mut response.choices {
            if let Some(Content::Text(text)) = &choice.delta.content {
                if !text.is_empty() {
                    let out = self.parser.feed(text);
                    if !out.content.is_empty() {
                        choice.delta.content = Some(Content::Text(out.content));
                        modified = true;
                    } else {
                        choice.delta.content = None;
                        modified = true;
                    }
                    if !out.thinking.is_empty() {
                        append_reasoning(&mut choice.delta.reasoning_content, &out.thinking);
                        modified = true;
                    }
                }
            }
        }
        modified
    }
}

fn append_reasoning(target: &mut Option<String>, text: &str) {
    match target {
        Some(existing) => existing.push_str(text),
        None => *target = Some(text.to_string()),
    }
}

/// 直通路径规范化器：操作原始 SSE 字节
pub struct PassthroughNormalizer {
    tag_parser: ThinkTagParser,
    /// Anthropic signature 缓存
    pending_signature: Option<String>,
    /// Anthropic thinking block 的 index
    signature_block_index: Option<u32>,
}

impl Default for PassthroughNormalizer {
    fn default() -> Self {
        Self::new()
    }
}

impl PassthroughNormalizer {
    pub fn new() -> Self {
        Self {
            tag_parser: ThinkTagParser::new(),
            pending_signature: None,
            signature_block_index: None,
        }
    }

    /// 处理单个 SSE 事件字节
    pub fn process_sse(&mut self, raw_bytes: &[u8], endpoint: &EndpointType) -> Vec<Vec<u8>> {
        match endpoint {
            EndpointType::OpenAiChat => self.process_openai_event(raw_bytes),
            EndpointType::Anthropic => self.process_anthropic_event(raw_bytes),
            _ => vec![raw_bytes.to_vec()],
        }
    }

    fn process_openai_event(&mut self, raw_bytes: &[u8]) -> Vec<Vec<u8>> {
        let text = match std::str::from_utf8(raw_bytes) {
            Ok(t) => t,
            Err(_) => return vec![raw_bytes.to_vec()],
        };

        let data_line = text
            .lines()
            .find(|l| l.trim_start().starts_with("data:"));
        let data = match data_line {
            Some(d) => d.trim_start().strip_prefix("data:").unwrap_or("").trim_start(),
            None => return vec![raw_bytes.to_vec()],
        };

        if data.is_empty() || data == "[DONE]" {
            return vec![raw_bytes.to_vec()];
        }

        // 快速预检：无 `<` 字节则不可能有 `<think/>` 标签，零拷贝透传
        if !data.contains('<') {
            return vec![raw_bytes.to_vec()];
        }

        let mut parsed: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => return vec![raw_bytes.to_vec()],
        };

        let mut content_modified = false;
        let mut reasoning_to_add: Option<String> = None;

        if let Some(choices) = parsed.get_mut("choices").and_then(|c| c.as_array_mut()) {
            for choice in choices {
                if let Some(delta) = choice.get_mut("delta").and_then(|d| d.as_object_mut()) {
                    if let Some(content_val) = delta.get("content").and_then(|c| c.as_str()) {
                        if !content_val.is_empty() {
                            let out = self.tag_parser.feed(content_val);
                            if !out.content.is_empty() {
                                delta.insert("content".to_string(), Value::String(out.content));
                            } else {
                                delta.remove("content");
                            }
                            content_modified = true;
                            if !out.thinking.is_empty() {
                                reasoning_to_add = Some(out.thinking);
                            }
                        }
                    }
                }
            }
        }

        if !content_modified {
            return vec![raw_bytes.to_vec()];
        }

        if let Some(thinking) = reasoning_to_add {
            if let Some(choices) = parsed.get_mut("choices").and_then(|c| c.as_array_mut()) {
                for choice in choices {
                    if let Some(delta) = choice.get_mut("delta").and_then(|d| d.as_object_mut()) {
                        match delta.get_mut("reasoning_content") {
                            Some(existing) => {
                                if let Some(s) = existing.as_str() {
                                    let mut new = s.to_string();
                                    new.push_str(&thinking);
                                    *existing = Value::String(new);
                                } else {
                                    delta.insert(
                                        "reasoning_content".to_string(),
                                        Value::String(thinking.clone()),
                                    );
                                }
                            }
                            None => {
                                delta.insert(
                                    "reasoning_content".to_string(),
                                    Value::String(thinking.clone()),
                                );
                            }
                        }
                    }
                }
            }
        }

        let new_data = match serde_json::to_string(&parsed) {
            Ok(s) => s,
            Err(_) => return vec![raw_bytes.to_vec()],
        };
        let new_event = format!("data: {}\n\n", new_data).into_bytes();
        vec![new_event]
    }

    fn process_anthropic_event(&mut self, raw_bytes: &[u8]) -> Vec<Vec<u8>> {
        let text = match std::str::from_utf8(raw_bytes) {
            Ok(t) => t,
            Err(_) => return vec![raw_bytes.to_vec()],
        };

        let event_type = text
            .lines()
            .find_map(|l| l.strip_prefix("event:").map(|v| v.trim().to_string()))
            .unwrap_or_default();

        let data_line = text.lines().find_map(|l| {
            let trimmed = l.trim_start();
            if trimmed.starts_with("data:") {
                Some(trimmed)
            } else {
                None
            }
        });

        if event_type == "content_block_start" {
            let data = data_line
                .and_then(|d| d.strip_prefix("data:"))
                .unwrap_or("")
                .trim_start();
            // 快速预检：data 中没有 "thinking" 字符串则不是 thinking block，跳过 JSON 解析
            if data.contains("thinking")
                && let Ok(parsed) = serde_json::from_str::<Value>(data)
            {
                let cb_type = parsed["content_block"]["type"].as_str().unwrap_or("");
                if cb_type == "thinking"
                    && let Some(sig) = parsed["content_block"]["signature"].as_str()
                {
                    self.pending_signature = Some(sig.to_string());
                    self.signature_block_index =
                        parsed["index"].as_u64().map(|i| i as u32);
                    // 移除 signature 后原样转发
                    let mut cleaned = parsed.clone();
                    if let Some(cb) = cleaned
                        .get_mut("content_block")
                        .and_then(|c| c.as_object_mut())
                    {
                        cb.remove("signature");
                    }
                    let new_data = match serde_json::to_string(&cleaned) {
                        Ok(s) => s,
                        Err(_) => return vec![raw_bytes.to_vec()],
                    };
                    let new_event = format!(
                        "event: content_block_start\ndata: {}\n\n",
                        new_data
                    )
                    .into_bytes();
                    return vec![new_event];
                }
            }
            return vec![raw_bytes.to_vec()];
        }

        if event_type == "content_block_stop" {
            if let Some(sig) = self.pending_signature.take() {
                let block_index = self.signature_block_index.take().unwrap_or(0);
                let sig_event = format!(
                    "event: content_block_delta\ndata: {}\n\n",
                    serde_json::json!({
                        "type": "content_block_delta",
                        "index": block_index,
                        "delta": {
                            "type": "signature_delta",
                            "signature": sig
                        }
                    })
                )
                .into_bytes();
                return vec![sig_event, raw_bytes.to_vec()];
            }
            return vec![raw_bytes.to_vec()];
        }

        vec![raw_bytes.to_vec()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_tag_in_one_chunk() {
        let mut p = ThinkTagParser::new();
        let out = p.feed("<think>hello</think> world");
        assert_eq!(out.content, " world");
        assert_eq!(out.thinking, "hello");
    }

    #[test]
    fn tag_split_across_chunks() {
        let mut p = ThinkTagParser::new();
        let out1 = p.feed("<think>hel");
        assert_eq!(out1.content, "");
        assert_eq!(out1.thinking, "");

        let out2 = p.feed("lo</think> world");
        assert_eq!(out2.content, " world");
        assert_eq!(out2.thinking, "hello");
    }

    #[test]
    fn tag_split_at_angle_bracket() {
        let mut p = ThinkTagParser::new();
        let out1 = p.feed("hello <thi");
        assert_eq!(out1.content, "hello ");
        assert_eq!(out1.thinking, "");

        let out2 = p.feed("nk>think content</think");
        assert_eq!(out2.content, "");
        assert_eq!(out2.thinking, "think content");

        let out3 = p.feed("> done");
        assert_eq!(out3.content, " done");
        assert_eq!(out3.thinking, "");
    }

    #[test]
    fn multiple_think_blocks() {
        let mut p = ThinkTagParser::new();
        // 多个 think 块在同一次 feed 中都会产出，thinking 累加
        let out = p.feed("<think>first</think> middle<think>second</think> end");
        assert_eq!(out.content, " middle end");
        assert_eq!(out.thinking, "first\nsecond");

        let out2 = p.feed("");
        assert_eq!(out2.content, "");
        assert_eq!(out2.thinking, "");
    }

    #[test]
    fn no_tags_passthrough_unchanged() {
        let mut p = ThinkTagParser::new();
        let out = p.feed("plain text without any tags");
        assert_eq!(out.content, "plain text without any tags");
        assert_eq!(out.thinking, "");
    }

    #[test]
    fn malformed_tag_recovers() {
        // <think without closing > should be treated as content
        let mut p = ThinkTagParser::new();
        let out = p.feed("hello <think without close");
        assert_eq!(out.content, "hello <think without close");
        assert_eq!(out.thinking, "");
    }

    #[test]
    fn flush_drains_pending() {
        let mut p = ThinkTagParser::new();
        p.feed("<think>thinking without close");
        let out = p.flush();
        // 未闭合的 thinking 视为正文
        assert_eq!(out.content, "thinking without close");
        assert_eq!(out.thinking, "");
    }

    #[test]
    fn real_minimax_m3_pattern() {
        // MiniMax 的实际格式：<think\n>content\n</think\n>（\n 在开/闭标签内）
        let mut p = ThinkTagParser::new();
        let out = p.feed(
            "<think\n>用户让我分析...让我先探索一下项目。\n</think\n>\n\n我来分析项目并查看 CI 报错情况。",
        );
        // 开标签内 \n 和闭标签内 \n 都被状态机消费
        assert_eq!(out.content, "\n\n我来分析项目并查看 CI 报错情况。");
        assert_eq!(out.thinking, "用户让我分析...让我先探索一下项目。\n");
    }

    #[test]
    fn debug_openclosing_newline() {
        let mut p = ThinkTagParser::new();
        // \n 在 <think 后、> 之前
        let out = p.feed("<think\n>hello</think>");
        assert_eq!(out.content, "");
        assert_eq!(out.thinking, "hello");
    }

    #[test]
    fn think_with_newline_inside_opening_tag() {
        // 一些厂商使用 <think\n> 而非 <think>>
        let mut p = ThinkTagParser::new();
        let out = p.feed("<think\n>hello</think> world");
        assert_eq!(out.content, " world");
        assert_eq!(out.thinking, "hello");
    }

    #[test]
    fn think_with_newline_inside_closing_tag() {
        let mut p = ThinkTagParser::new();
        let out = p.feed("<think>hello</think\n> world");
        assert_eq!(out.content, " world");
        assert_eq!(out.thinking, "hello");
    }

    #[test]
    fn openai_passthrough_extracts_think_tag() {
        let mut norm = PassthroughNormalizer::new();
        let raw = b"data: {\"choices\":[{\"delta\":{\"content\":\"<think>hidden</think> visible\"}}]}\n\n";
        let result = norm.process_sse(raw, &EndpointType::OpenAiChat);
        assert_eq!(result.len(), 1);
        let output = String::from_utf8_lossy(&result[0]);
        assert!(
            output.contains("\"content\":\" visible\""),
            "output: {}",
            output
        );
        assert!(
            output.contains("\"reasoning_content\":\"hidden\""),
            "output: {}",
            output
        );
    }

    #[test]
    fn openai_passthrough_unchanged_when_no_tags() {
        let mut norm = PassthroughNormalizer::new();
        let raw = b"data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\n";
        let result = norm.process_sse(raw, &EndpointType::OpenAiChat);
        assert_eq!(result, vec![raw.to_vec()]);
    }

    #[test]
    fn anthropic_signature_extracted_from_start() {
        let mut norm = PassthroughNormalizer::new();
        let start = b"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"signature\":\"3d93fc8d26ce411e81f4c34d\"}}\n\n";
        let result = norm.process_sse(start, &EndpointType::Anthropic);
        assert_eq!(result.len(), 1);
        let output = String::from_utf8_lossy(&result[0]);
        assert!(!output.contains("signature"), "output: {}", output);
    }

    #[test]
    fn anthropic_signature_injected_before_stop() {
        let mut norm = PassthroughNormalizer::new();
        let start = b"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"signature\":\"3d93fc8d26ce411e81f4c34d\"}}\n\n";
        norm.process_sse(start, &EndpointType::Anthropic);

        let stop = b"event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n";
        let result = norm.process_sse(stop, &EndpointType::Anthropic);
        assert_eq!(result.len(), 2);
        let sig_event = String::from_utf8_lossy(&result[0]);
        assert!(
            sig_event.contains("signature_delta"),
            "got: {}",
            sig_event
        );
        assert!(
            sig_event.contains("3d93fc8d26ce411e81f4c34d"),
            "got: {}",
            sig_event
        );

        let stop_event = String::from_utf8_lossy(&result[1]);
        assert!(
            stop_event.contains("content_block_stop"),
            "got: {}",
            stop_event
        );
    }

    #[test]
    fn anthropic_no_signature_passthrough_unchanged() {
        let mut norm = PassthroughNormalizer::new();
        let start = b"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\"}}\n\n";
        let result = norm.process_sse(start, &EndpointType::Anthropic);
        assert_eq!(result, vec![start.to_vec()]);
    }

    #[test]
    fn anthropic_text_block_unchanged() {
        let mut norm = PassthroughNormalizer::new();
        let start = b"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n";
        let result = norm.process_sse(start, &EndpointType::Anthropic);
        assert_eq!(result, vec![start.to_vec()]);
    }

    #[test]
    fn conversion_path_extractor() {
        use crate::protocol::model::{Message, Role, StreamChoice};

        let mut ext = ThinkingTagExtractor::new();
        let mut resp = LlmStreamResponse {
            id: String::new(),
            object: "chat.completion.chunk".to_string(),
            created: 0,
            model: String::new(),
            choices: vec![StreamChoice {
                index: 0,
                delta: Message {
                    role: Role::Assistant,
                    content: Some(Content::Text(
                        "<think>thinking</think> actual".to_string(),
                    )),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                    reasoning_content: None,
                    cache_control: None,
                },
                finish_reason: None,
                thinking_signature: None,
            }],
            usage: None,
            system_fingerprint: None,
        };
        let modified = ext.extract(&mut resp);
        assert!(modified);
        match &resp.choices[0].delta.content {
            Some(Content::Text(s)) => assert_eq!(s, " actual"),
            other => panic!("expected Text(\" actual\"), got {:?}", other),
        }
        assert_eq!(
            resp.choices[0].delta.reasoning_content,
            Some("thinking".to_string())
        );
    }
}
