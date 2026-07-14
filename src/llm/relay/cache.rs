use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::relay::channel::ChannelInfo;
use crate::scheduler::selector::RouteInfo;

/// 缓存大小限制
const CACHE_MAX_SIZE: usize = 1000;

/// 渠道/分组缓存（含模型反向索引）
#[derive(Clone)]
pub struct ProxyCache {
    channels: Arc<RwLock<HashMap<String, ChannelInfo>>>,
    routes: Arc<RwLock<HashMap<String, RouteInfo>>>,
    model_index: Arc<RwLock<HashMap<String, Vec<String>>>>,
    compiled_regex: Arc<RwLock<HashMap<String, regex::Regex>>>,
}

impl Default for ProxyCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ProxyCache {
    pub fn new() -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            routes: Arc::new(RwLock::new(HashMap::new())),
            model_index: Arc::new(RwLock::new(HashMap::new())),
            compiled_regex: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 获取缓存的渠道
    pub async fn get_channel(&self, id: &str) -> Option<ChannelInfo> {
        let cache = self.channels.read().await;
        cache.get(id).cloned()
    }

    /// 设置渠道缓存（超过限制时清除最旧条目）
    pub async fn set_channel(&self, channel: ChannelInfo) {
        let mut cache = self.channels.write().await;

        // 如果渠道已存在，先从旧模型的索引中清除此 channel_id
        if let Some(old_ch) = cache.get(&channel.id) {
            let mut idx = self.model_index.write().await;
            for model in &old_ch.models {
                if let Some(ids) = idx.get_mut(model) {
                    ids.retain(|id| id != &channel.id);
                }
            }
        }

        if cache.len() >= CACHE_MAX_SIZE
            && !cache.contains_key(&channel.id)
            && let Some(oldest_key) = cache.keys().next().cloned()
        {
            let mut idx = self.model_index.write().await;
            if let Some(old_ch) = cache.get(&oldest_key) {
                for model in &old_ch.models {
                    if let Some(ids) = idx.get_mut(model) {
                        ids.retain(|id| id != &oldest_key);
                    }
                }
            }
            cache.remove(&oldest_key);
        }

        // 更新模型反向索引
        {
            let mut idx = self.model_index.write().await;
            for model in &channel.models {
                idx.entry(model.clone())
                    .or_default()
                    .push(channel.id.clone());
            }
        }

        cache.insert(channel.id.clone(), channel);
    }

    /// 清除渠道缓存
    pub async fn invalidate_channel(&self, id: &str) {
        let mut cache = self.channels.write().await;
        if let Some(ch) = cache.remove(id) {
            let mut idx = self.model_index.write().await;
            for model in &ch.models {
                if let Some(ids) = idx.get_mut(model) {
                    ids.retain(|cid| cid != id);
                }
            }
        }
    }

    /// 清除所有渠道缓存
    pub async fn invalidate_all_channels(&self) {
        let mut cache = self.channels.write().await;
        cache.clear();
        self.model_index.write().await.clear();
    }

    /// 获取缓存的分组
    pub async fn get_group(&self, name: &str) -> Option<RouteInfo> {
        let cache = self.routes.read().await;
        cache.get(name).cloned()
    }

    /// 设置分组缓存（超过限制时清除最旧条目）
    pub async fn set_group(&self, group: RouteInfo) {
        let mut cache = self.routes.write().await;
        if cache.len() >= CACHE_MAX_SIZE
            && let Some(oldest_key) = cache.keys().next().cloned()
        {
            cache.remove(&oldest_key);
        }
        cache.insert(group.name.clone(), group);
    }

    /// 清除所有分组缓存
    pub async fn invalidate_all_routes(&self) {
        let mut cache = self.routes.write().await;
        cache.clear();
    }

    /// 查找包含指定模型的渠道 ID 列表
    #[allow(dead_code)]
    pub async fn find_channels_by_model(&self, model: &str) -> Vec<String> {
        let idx = self.model_index.read().await;
        idx.get(model).cloned().unwrap_or_default()
    }

    /// 获取或编译正则（缓存编译结果）
    pub(crate) async fn get_compiled_regex(&self, pattern: &str) -> Option<regex::Regex> {
        {
            let cache = self.compiled_regex.read().await;
            if let Some(re) = cache.get(pattern) {
                return Some(re.clone());
            }
        }
        let re = regex::Regex::new(pattern).ok()?;
        let mut cache = self.compiled_regex.write().await;
        cache.insert(pattern.to_string(), re.clone());
        Some(re)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::handlers::admin::channels::EndpointType;

    fn sample_channel(id: &str, models: Vec<&str>) -> ChannelInfo {
        ChannelInfo {
            id: id.into(),
            name: id.into(),
            api_keys: vec![],
            endpoints: vec![crate::api::handlers::admin::channels::EndpointConfig {
                base_url: "https://example.com".into(),
                endpoint_type: EndpointType::OpenAiChat,
                enabled: true,
                headers: vec![],
            }],
            models: models.into_iter().map(String::from).collect(),
            timeout_secs: 300,
            max_concurrency: 0,
            failure_threshold: 3,
            blacklist_minutes: 10,
        }
    }

    fn sample_group(name: &str) -> RouteInfo {
        RouteInfo {
            id: "g-1".into(),
            name: name.into(),
            items: vec![],
        }
    }

    #[tokio::test]
    async fn set_channel_updates_model_index() {
        let cache = ProxyCache::new();
        cache
            .set_channel(sample_channel("ch-a", vec!["gpt-4", "gpt-3.5"]))
            .await;
        assert_eq!(cache.find_channels_by_model("gpt-4").await, vec!["ch-a"]);
        assert_eq!(cache.find_channels_by_model("gpt-3.5").await, vec!["ch-a"]);
        assert!(cache.find_channels_by_model("claude").await.is_empty());
    }

    #[tokio::test]
    async fn invalidate_channel_removes_from_index() {
        let cache = ProxyCache::new();
        cache
            .set_channel(sample_channel("ch-a", vec!["gpt-4"]))
            .await;
        cache.invalidate_channel("ch-a").await;
        assert!(cache.find_channels_by_model("gpt-4").await.is_empty());
        assert!(cache.get_channel("ch-a").await.is_none());
    }

    #[tokio::test]
    async fn invalidate_all_channels_clears_index() {
        let cache = ProxyCache::new();
        cache
            .set_channel(sample_channel("ch-a", vec!["gpt-4"]))
            .await;
        cache
            .set_channel(sample_channel("ch-b", vec!["claude"]))
            .await;
        cache.invalidate_all_channels().await;
        assert!(cache.find_channels_by_model("gpt-4").await.is_empty());
        assert!(cache.find_channels_by_model("claude").await.is_empty());
    }

    #[tokio::test]
    async fn set_channel_replaces_existing_entry() {
        let cache = ProxyCache::new();
        cache
            .set_channel(sample_channel("ch-a", vec!["gpt-4"]))
            .await;
        cache
            .set_channel(sample_channel("ch-a", vec!["gpt-3.5"]))
            .await;
        // 覆盖时旧 model 索引应被清理
        assert!(cache.find_channels_by_model("gpt-4").await.is_empty());
        assert_eq!(cache.find_channels_by_model("gpt-3.5").await, vec!["ch-a"]);
        let stored = cache.get_channel("ch-a").await.unwrap();
        assert_eq!(stored.models, vec!["gpt-3.5".to_string()]);
    }

    #[tokio::test]
    async fn group_cache_roundtrip() {
        let cache = ProxyCache::new();
        cache.set_group(sample_group("grp-1")).await;
        let got = cache.get_group("grp-1").await;
        assert!(got.is_some());
        assert_eq!(got.unwrap().name, "grp-1");
        assert!(cache.get_group("missing").await.is_none());
    }

    #[tokio::test]
    async fn invalidate_all_groups_clears() {
        let cache = ProxyCache::new();
        cache.set_group(sample_group("g-1")).await;
        cache.invalidate_all_routes().await;
        assert!(cache.get_group("g-1").await.is_none());
    }

    #[tokio::test]
    async fn compiled_regex_caches() {
        let cache = ProxyCache::new();
        let re1 = cache.get_compiled_regex(r"^gpt-").await.unwrap();
        let re2 = cache.get_compiled_regex(r"^gpt-").await.unwrap();
        assert!(re1.is_match("gpt-4"));
        assert!(re2.is_match("gpt-3.5"));
    }

    #[tokio::test]
    async fn compiled_regex_invalid_returns_none() {
        let cache = ProxyCache::new();
        assert!(cache.get_compiled_regex(r"(unclosed").await.is_none());
    }
}
