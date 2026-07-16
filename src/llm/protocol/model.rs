use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 角色
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    #[default]
    User,
    System,
    Assistant,
    Tool,
    Developer,
}

/// 缓存控制（Anthropic prompt caching）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheControl {
    #[serde(rename = "type")]
    pub cache_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl: Option<String>,
}

/// 内容类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    ImageUrl {
        image_url: ImageUrl,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    InputAudio {
        input_audio: InputAudio,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_call_id: String,
        content: String,
    },
}

/// 图片 URL
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrl {
    pub url: String,
    pub detail: Option<String>,
}

/// 输入音频
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputAudio {
    pub data: String,
    pub format: String,
}

/// 内容（字符串或数组）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Parts(Vec<ContentPart>),
}

/// 工具调用
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCall,
}

/// 函数调用
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

/// 流式函数调用 delta（OpenAI 流式：name/arguments 均可空，由客户端按 `index` 分片累积）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StreamFunctionDelta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

/// 流式工具调用 delta（OpenAI 标准：`index` 必填，客户端按 index 拼接分片；
/// id/type/function 可空——首帧带 id+name，后续帧仅 arguments）。
/// `index` 用 `#[serde(default)]`：上游带 index 就读入，不带则 default 0（兼容智谱等非标准上游）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamToolCallDelta {
    #[serde(default)]
    pub index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub call_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function: Option<StreamFunctionDelta>,
}

/// 消息
///
/// 注意: `role` 字段在流式 delta 中可能缺失（只有第一个 chunk 包含 role），
/// 因此使用 `#[serde(default)]` 使其反序列化时不报错。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    #[serde(default)]
    pub role: Role,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

/// 流式 delta（区别于非流式 `Message`）：
/// - `role` 可空（流式通常仅首帧带 role；缺失时不再被 default 成 `user`）
/// - `tool_calls` 带 OpenAI 标准的 `index`（见 `StreamToolCallDelta`）
/// - 去掉 message-only 的 `name`/`tool_call_id`/`cache_control`（流式不用）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StreamDelta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Content>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<StreamToolCallDelta>>,
}

/// 工具定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionDefinition,
}

/// 函数定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDefinition {
    pub name: String,
    pub description: Option<String>,
    pub parameters: Option<serde_json::Value>,
}

/// 工具选择
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolChoice {
    None,
    Auto,
    Required,
    Function { function: FunctionName },
}

/// 函数名称
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionName {
    pub name: String,
}

/// 停止原因
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    FunctionCall,
}

/// 使用量
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens_details: Option<CompletionTokensDetails>,
}

/// Prompt Token 详情
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PromptTokensDetails {
    pub cached_tokens: u32,
}

/// Completion Token 详情
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompletionTokensDetails {
    pub reasoning_tokens: u32,
}

/// 统一请求模型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    /// 额外字段（不发送给 LLM）
    #[serde(skip)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// 选择
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Choice {
    pub index: u32,
    pub message: Message,
    pub finish_reason: Option<FinishReason>,
}

/// 流式选择
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamChoice {
    pub index: u32,
    pub delta: StreamDelta,
    pub finish_reason: Option<FinishReason>,
}

/// 统一响应模型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<Choice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<String>,
}

/// 流式响应模型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmStreamResponse {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<StreamChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<String>,
}

impl LlmResponse {
    /// 获取完成原因
    pub fn finish_reason(&self) -> Option<&FinishReason> {
        self.choices.first().and_then(|c| c.finish_reason.as_ref())
    }
}

impl LlmStreamResponse {
    /// 获取第一个选择
    pub fn first_choice(&self) -> Option<&StreamChoice> {
        self.choices.first()
    }
}

/// 解析后的 SSE 事件（参考 AxonHub StreamEvent）
pub struct SseEvent {
    pub event_type: String,
    pub data: String,
}

/// 从 SSE 文本中解析出 event_type 和 data
pub fn parse_sse_event(text: &str) -> SseEvent {
    let mut event_type = String::new();
    let mut data = String::new();
    for line in text.lines() {
        if let Some(stripped) = line.strip_prefix("event: ") {
            event_type = stripped.trim().to_string();
        } else if let Some(stripped) = line.strip_prefix("data: ") {
            data = stripped.trim_start().to_string();
        }
    }
    SseEvent { event_type, data }
}
