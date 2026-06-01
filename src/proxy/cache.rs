use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::{ChannelInfo, GroupInfo};

/// 缓存大小限制
const CACHE_MAX_SIZE: usize = 1000;

/// 渠道/分组缓存（含模型反向索引）
#[derive(Clone)]
pub struct ProxyCache {
    channels: Arc<RwLock<HashMap<String, ChannelInfo>>>,
    groups: Arc<RwLock<HashMap<String, GroupInfo>>>,
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
            groups: Arc::new(RwLock::new(HashMap::new())),
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

        if cache.len() >= CACHE_MAX_SIZE
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
    pub async fn get_group(&self, name: &str) -> Option<GroupInfo> {
        let cache = self.groups.read().await;
        cache.get(name).cloned()
    }

    /// 设置分组缓存（超过限制时清除最旧条目）
    pub async fn set_group(&self, group: GroupInfo) {
        let mut cache = self.groups.write().await;
        if cache.len() >= CACHE_MAX_SIZE
            && let Some(oldest_key) = cache.keys().next().cloned()
        {
            cache.remove(&oldest_key);
        }
        cache.insert(group.name.clone(), group);
    }

    /// 清除所有分组缓存
    pub async fn invalidate_all_groups(&self) {
        let mut cache = self.groups.write().await;
        cache.clear();
    }

    /// 查找包含指定模型的渠道 ID 列表
    pub async fn find_channels_by_model(&self, model: &str) -> Vec<String> {
        let idx = self.model_index.read().await;
        idx.get(model).cloned().unwrap_or_default()
    }

    /// 获取或编译正则（缓存编译结果）
    pub(super) async fn get_compiled_regex(&self, pattern: &str) -> Option<regex::Regex> {
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
