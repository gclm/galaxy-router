//! 渠道领域类型（跨层基础类型）。
//!
//! 从 `api/handlers/admin/channels/types.rs` 归位（B1-C1）。
//! 被 `llm/`（relay/protocol/scheduler/plugin）、proxy handler、repository 共享，
//! 是真正的跨层契约，故入 domain。

use serde::{Deserialize, Serialize};

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

/// 解析渠道的 api_keys JSON 字段为 Vec<UpstreamApiKey>（启动加载渠道 + 调度任务用）
pub fn parse_api_keys(json_str: &str) -> Vec<UpstreamApiKey> {
    serde_json::from_str(json_str).unwrap_or_default()
}

/// 自定义请求头
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CustomHeader {
    pub key: String,
    pub value: String,
}

/// 渠道聚合根
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

fn default_true() -> bool {
    true
}

/// 渠道运行时信息（relay 调度用；B3-C0 从 llm/relay/channel 归位）
#[derive(Debug, Clone)]
pub struct ChannelInfo {
    pub id: String,
    pub name: String,
    pub api_keys: Vec<UpstreamApiKey>,
    pub endpoints: Vec<EndpointConfig>,
    pub models: Vec<String>,
    /// 单次请求超时（秒），默认 300
    pub timeout_secs: u64,
    /// 最大并发请求数（0=不限制）
    pub max_concurrency: u32,
    /// 渠道级黑名单触发阈值（连续失败次数）。默认 3
    pub failure_threshold: u64,
    /// 渠道级黑名单时长（分钟）。默认 10
    pub blacklist_minutes: i64,
}

impl ChannelInfo {
    /// 获取启用的 API Key 列表
    pub fn enabled_api_keys(&self) -> Vec<&UpstreamApiKey> {
        self.api_keys.iter().filter(|k| k.enabled).collect()
    }

    /// 查找指定类型的端点（跳过已禁用的）
    pub fn find_endpoint(&self, endpoint_type: &EndpointType) -> Option<EndpointConfig> {
        self.endpoints
            .iter()
            .find(|e| e.enabled && e.endpoint_type == *endpoint_type)
            .cloned()
    }

    /// 是否有任意启用的端点（用于候选筛选，不要求协议匹配）
    pub fn has_any_endpoint(&self) -> bool {
        self.endpoints.iter().any(|e| e.enabled)
    }

    /// 选择最佳上游端点：优先精确匹配，fallback 到任意可用端点（由 pipeline 做协议转换）
    pub fn find_best_endpoint(&self, endpoint_type: &EndpointType) -> Option<EndpointConfig> {
        // 精确匹配：无需转换
        if let Some(ep) = self.find_endpoint(endpoint_type) {
            return Some(ep);
        }
        // Fallback：优先 openai_chat（最通用），其次 anthropic，最后任意
        self.endpoints
            .iter()
            .find(|e| e.enabled && e.endpoint_type == EndpointType::OpenAiChat)
            .or_else(|| {
                self.endpoints
                    .iter()
                    .find(|e| e.enabled && e.endpoint_type == EndpointType::Anthropic)
            })
            .or_else(|| self.endpoints.iter().find(|e| e.enabled))
            .cloned()
    }

    /// 生成上游 Key 的显示 hint（优先 note，否则截断）
    pub fn key_hint(&self, key: &str) -> String {
        if let Some(ak) = self
            .api_keys
            .iter()
            .find(|ak| ak.key == key && !ak.note.is_empty())
        {
            return ak.note.clone();
        }
        if key.len() > 12 {
            format!("{}...{}", &key[..8], &key[key.len() - 4..])
        } else if key.len() > 4 {
            format!("{}...{}", &key[..3], &key[key.len() - 2..])
        } else {
            key.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_channel() -> ChannelInfo {
        ChannelInfo {
            id: "ch-1".into(),
            name: "test".into(),
            api_keys: vec![
                UpstreamApiKey {
                    key: "sk-abc".into(),
                    note: "primary".into(),
                    enabled: true,
                },
                UpstreamApiKey {
                    key: "sk-xyz".into(),
                    note: "".into(),
                    enabled: false,
                },
            ],
            endpoints: vec![
                EndpointConfig {
                    base_url: "https://api.openai.com".into(),
                    endpoint_type: EndpointType::OpenAiChat,
                    enabled: true,
                    headers: vec![],
                },
                EndpointConfig {
                    base_url: "https://api.openai.com".into(),
                    endpoint_type: EndpointType::Anthropic,
                    enabled: false,
                    headers: vec![],
                },
            ],
            models: vec!["gpt-4".into()],
            timeout_secs: 300,
            max_concurrency: 0,
            failure_threshold: 3,
            blacklist_minutes: 10,
        }
    }

    #[test]
    fn enabled_api_keys_filters_disabled() {
        let ch = sample_channel();
        let keys = ch.enabled_api_keys();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].key, "sk-abc");
    }

    #[test]
    fn find_endpoint_skips_disabled() {
        let ch = sample_channel();
        assert!(ch.find_endpoint(&EndpointType::OpenAiChat).is_some());
        assert!(ch.find_endpoint(&EndpointType::Anthropic).is_none());
        assert!(ch.find_endpoint(&EndpointType::OpenAiResponse).is_none());
    }

    #[test]
    fn key_hint_prefers_note() {
        let ch = sample_channel();
        assert_eq!(ch.key_hint("sk-abc"), "primary");
    }

    #[test]
    fn key_hint_truncates_long_key_without_note() {
        let ch = sample_channel();
        let key = "sk-abcdefghijklmnop";
        let hint = ch.key_hint(key);
        assert_eq!(hint, "sk-abcde...mnop");
    }

    #[test]
    fn key_hint_returns_short_key_unchanged() {
        let ch = sample_channel();
        assert_eq!(ch.key_hint("sk"), "sk");
    }
}
