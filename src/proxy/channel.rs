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
