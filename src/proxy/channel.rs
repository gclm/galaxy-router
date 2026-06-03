use crate::api::handlers::admin::channels::{
    CustomHeader, EndpointConfig, EndpointType, UpstreamApiKey,
};

/// 渠道信息
#[derive(Debug, Clone)]
pub struct ChannelInfo {
    pub id: String,
    pub name: String,
    pub api_keys: Vec<UpstreamApiKey>,
    pub endpoints: Vec<EndpointConfig>,
    pub models: Vec<String>,
    pub custom_headers: Vec<CustomHeader>,
    /// 单次请求超时（秒），默认 300
    pub timeout_secs: u64,
    /// 最大并发请求数（0=不限制）
    pub max_concurrency: u32,
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
    use crate::api::handlers::admin::channels::{EndpointType, UpstreamApiKey};

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
                },
                EndpointConfig {
                    base_url: "https://api.openai.com".into(),
                    endpoint_type: EndpointType::Anthropic,
                    enabled: false,
                },
            ],
            models: vec!["gpt-4".into()],
            custom_headers: vec![],
            timeout_secs: 300,
            max_concurrency: 0,
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
