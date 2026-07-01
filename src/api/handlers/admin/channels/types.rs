use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(sqlx::FromRow)]
pub(crate) struct ChannelRow {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) api_keys: String,
    pub(crate) endpoints: String,
    pub(crate) models: String,
    pub(crate) rate_limit_rpm: Option<i32>,
    pub(crate) rate_limit_tpm: Option<i32>,
    pub(crate) failure_threshold: i32,
    pub(crate) blacklist_minutes: i32,
    pub(crate) concurrency: i32,
    pub(crate) timeout_secs: i32,
    pub(crate) max_concurrency: i32,
    pub(crate) enabled: bool,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

/// 列表查询参数
#[derive(Debug, Deserialize)]
pub struct ListChannelsQuery {
    pub search: Option<String>,
    pub status: Option<String>,
    pub sort_by: Option<String>,
    pub sort_order: Option<String>,
    pub page: Option<i32>,
    pub page_size: Option<i32>,
}

/// 分页响应
#[derive(Debug, Serialize)]
pub struct PaginatedResponse<T: Serialize> {
    pub items: Vec<T>,
    pub total: i64,
}

/// 端点类型
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EndpointType {
    #[serde(rename = "openai_chat")]
    OpenAiChat,
    #[serde(rename = "openai_response")]
    OpenAiResponse,
    Anthropic,
    Gemini,
    #[serde(rename = "openai_embedding")]
    OpenAiEmbedding,
    #[serde(rename = "openai_images")]
    OpenAiImages,
}

impl EndpointType {
    /// 获取端点路径
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAiChat => "openai_chat",
            Self::OpenAiResponse => "openai_response",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
            Self::OpenAiEmbedding => "openai_embedding",
            Self::OpenAiImages => "openai_images",
        }
    }

    pub fn path(&self) -> &'static str {
        match self {
            Self::OpenAiChat => "/chat/completions",
            Self::OpenAiResponse => "/responses",
            Self::Anthropic => "/messages",
            Self::Gemini => "/models/{model}:generateContent",
            Self::OpenAiEmbedding => "/embeddings",
            Self::OpenAiImages => "/images/generations",
        }
    }
}

/// 端点配置
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EndpointConfig {
    #[serde(rename = "type")]
    pub endpoint_type: EndpointType,
    pub base_url: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 端点级自定义请求头（insert 覆盖客户端，用于按协议配不同 User-Agent 等）
    #[serde(default)]
    pub headers: Vec<CustomHeader>,
    /// 端点级扩展配置（thinking.extract_tags/fix_signature 等，按端点配）
    #[serde(default)]
    pub extras: Option<serde_json::Map<String, serde_json::Value>>,
}

/// 上游 API Key
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UpstreamApiKey {
    pub key: String,
    #[serde(default)]
    pub note: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// 自定义请求头
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CustomHeader {
    pub key: String,
    pub value: String,
}

/// 渠道
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub api_keys: Vec<UpstreamApiKey>,
    pub endpoints: Vec<EndpointConfig>,
    pub models: Vec<String>,
    pub rate_limit_rpm: Option<i32>,
    pub rate_limit_tpm: Option<i32>,
    pub failure_threshold: i32,
    pub blacklist_minutes: i32,
    pub concurrency: i32,
    pub timeout_secs: i32,
    pub max_concurrency: i32,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// 创建渠道请求
#[derive(Debug, Deserialize)]
pub struct CreateChannelRequest {
    pub name: String,
    pub api_keys: Vec<UpstreamApiKey>,
    pub endpoints: Vec<EndpointConfig>,
    pub models: Option<Vec<String>>,
    pub rate_limit_rpm: Option<i32>,
    pub rate_limit_tpm: Option<i32>,
    pub failure_threshold: Option<i32>,
    pub blacklist_minutes: Option<i32>,
    pub concurrency: Option<i32>,
    pub timeout_secs: Option<i32>,
    pub max_concurrency: Option<i32>,
    pub enabled: Option<bool>,
}

/// 更新渠道请求
#[derive(Debug, Deserialize)]
pub struct UpdateChannelRequest {
    pub name: Option<String>,
    pub api_keys: Option<Vec<UpstreamApiKey>>,
    pub endpoints: Option<Vec<EndpointConfig>>,
    pub models: Option<Vec<String>>,
    pub rate_limit_rpm: Option<i32>,
    pub rate_limit_tpm: Option<i32>,
    pub failure_threshold: Option<i32>,
    pub blacklist_minutes: Option<i32>,
    pub concurrency: Option<i32>,
    pub timeout_secs: Option<i32>,
    pub max_concurrency: Option<i32>,
    pub enabled: Option<bool>,
}

/// 渠道状态
#[derive(Clone)]
pub struct ChannelState {
    pub pool: SqlitePool,
    pub cache: crate::relay::cache::ProxyCache,
    pub http_client: reqwest::Client,
    pub timezone_offset: i32,
}

/// 测试渠道请求
#[derive(Debug, Deserialize)]
pub struct TestChannelRequest {
    pub model: String,
    pub test_protocol: String,
    pub api_key: String,
    pub stream: Option<bool>,
    pub user_agent: Option<String>,
}

/// 测试渠道响应
#[derive(Debug, Serialize)]
pub struct TestChannelResponse {
    pub success: bool,
    pub message: String,
    pub latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_to_first_token_ms: Option<u64>,
    pub input_prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u64>,
}

/// 检测渠道 quirks 请求（所有参数可选）
#[derive(Debug, Deserialize, Default)]
pub struct DetectRequest {
    /// 指定 endpoint 列表（如 `["openai_chat", "anthropic"]`）；不传则测所有启用端点
    pub endpoints: Option<Vec<String>>,
    /// 指定 API key；不传则用第一个启用的
    pub api_key: Option<String>,
    /// 指定 model；不传则用渠道 models 第一个
    pub model: Option<String>,
}

/// 单个 endpoint 的检测结果
#[derive(Debug, Serialize, Clone)]
pub struct EndpointDetection {
    pub endpoint: String,
    /// 推荐的开关（key 是如 "thinking.extract_tags"）
    pub recommendations: std::collections::HashMap<String, bool>,
    /// 检测证据描述
    pub evidence: String,
    /// 响应样本（截断到 200 字符）
    pub sample: String,
}

/// 检测渠道 quirks 响应
#[derive(Debug, Serialize)]
pub struct DetectResponse {
    /// 渠道级合并推荐（任一 endpoint 建议开启则 true）
    pub recommendations: std::collections::HashMap<String, bool>,
    /// 每个 endpoint 的详细检测结果
    pub endpoint_results: Vec<EndpointDetection>,
}
