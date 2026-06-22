pub use crate::scheduler::selector::{GroupInfo, GroupItemInfo};

use crate::api::handlers::admin::channels::{
    CustomHeader, EndpointConfig, UpstreamApiKey, parse_api_keys,
};
use crate::metrics::model::ModelRegistry;
use crate::metrics::recorder::StatsRecorder;
use crate::relay::cache::ProxyCache;
use crate::relay::channel::ChannelInfo;
use crate::error::proxy::ProxyError;
use crate::relay::queue::RequestQueue;
use crate::relay::ratelimit::RateLimiter;
use crate::scheduler::state::LoadBalancerState;
use sqlx::SqlitePool;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// 代理状态
#[derive(Clone)]
pub struct ProxyState {
    pub pool: SqlitePool,
    pub http_client: reqwest::Client,
    pub lb_state: LoadBalancerState,
    pub stats_recorder: StatsRecorder,
    pub model_registry: ModelRegistry,
    pub cache: ProxyCache,
    pub rate_limiter: RateLimiter,
    pub queue: Option<RequestQueue>,
    key_counter: Arc<AtomicU64>,
}

/// 代理成功结果（非流式）
pub struct ProxySuccess {
    pub status: StatusCode,
    pub body: Vec<u8>,
}

use axum::http::StatusCode;

impl ProxyState {
    pub async fn new(pool: SqlitePool, model_registry: ModelRegistry) -> Self {
        let proxy_enabled: bool = sqlx::query_scalar::<_, String>(
            "SELECT value FROM settings WHERE key = 'proxy.enabled'",
        )
        .fetch_optional(&pool)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(false);

        let proxy_url = if proxy_enabled {
            sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = 'proxy.url'")
                .fetch_optional(&pool)
                .await
                .ok()
                .flatten()
                .filter(|v| !v.is_empty())
        } else {
            None
        };

        let mut client_builder =
            reqwest::Client::builder().timeout(std::time::Duration::from_secs(300));

        if let Some(url) = proxy_url {
            match reqwest::Proxy::all(&url) {
                Ok(proxy) => {
                    tracing::info!("上游代理已启用: {}", url);
                    client_builder = client_builder.proxy(proxy);
                }
                Err(e) => {
                    tracing::warn!("代理配置无效，忽略代理: {}", e);
                    client_builder = client_builder.no_proxy();
                }
            }
        } else {
            client_builder = client_builder.no_proxy();
        }

        let http_client = client_builder
            .build()
            .expect("Failed to create HTTP client");

        Self {
            stats_recorder: StatsRecorder::new(pool.clone()),
            model_registry,
            cache: ProxyCache::new(),
            rate_limiter: RateLimiter::new(),
            queue: None,
            pool,
            http_client,
            lb_state: LoadBalancerState::new(),
            key_counter: Arc::new(AtomicU64::new(0)),
        }
    }

    /// 设置请求队列
    pub fn with_queue(mut self, max_queue_size: usize, timeout_secs: u64) -> Self {
        self.queue = Some(RequestQueue::new(max_queue_size, timeout_secs));
        self
    }

    /// 根据名称查找分组（带缓存）
    pub(crate) async fn find_group_by_name(&self, name: &str) -> Result<Option<GroupInfo>, ProxyError> {
        // 1. 检查缓存
        if let Some(group) = self.cache.get_group(name).await {
            return Ok(Some(group));
        }

        // 2. 缓存未命中，查询数据库
        let result = sqlx::query_as::<_, (String, String)>(
            "SELECT id, name FROM groups WHERE name = ? AND enabled = 1",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        match result {
            Some((id, name)) => {
                let items = self.get_group_items(&id).await?;
                let group = GroupInfo {
                    id,
                    name: name.clone(),
                    items,
                };
                // 3. 写入缓存
                self.cache.set_group(group.clone()).await;
                Ok(Some(group))
            }
            None => Ok(None),
        }
    }

    /// 根据正则查找分组
    pub(crate) async fn find_group_by_regex(&self, model: &str) -> Result<Option<GroupInfo>, ProxyError> {
        let groups = sqlx::query_as::<_, (String, String, Option<String>)>(
            "SELECT id, name, match_regex FROM groups WHERE enabled = 1 AND match_regex IS NOT NULL"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        for (id, name, match_regex) in groups {
            if let Some(pattern) = match_regex
                && let Some(re) = self.cache.get_compiled_regex(&pattern).await
                && re.is_match(model)
            {
                let items = self.get_group_items(&id).await?;
                return Ok(Some(GroupInfo { id, name, items }));
            }
        }

        Ok(None)
    }

    /// 获取分组项
    async fn get_group_items(&self, group_id: &str) -> Result<Vec<GroupItemInfo>, ProxyError> {
        let items = sqlx::query_as::<_, (String, String, i32, i32)>(
            "SELECT channel_id, model_name, priority, weight FROM group_items WHERE group_id = ? ORDER BY priority ASC, weight DESC"
        )
        .bind(group_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        Ok(items
            .into_iter()
            .map(|(channel_id, model_name, priority, weight)| GroupItemInfo {
                channel_id,
                model_name,
                priority,
                weight,
            })
            .collect())
    }
}

/// 获取渠道信息（带缓存）
impl ProxyState {
    pub(crate) async fn get_channel(&self, channel_id: &str) -> Result<ChannelInfo, ProxyError> {
        // 1. 检查缓存
        if let Some(channel) = self.cache.get_channel(channel_id).await {
            return Ok(channel);
        }

        // 2. 缓存未命中，查询数据库
        let result = sqlx::query_as::<_, (String, String, String, String, String, String, i32, i32, String, i32, i32)>(
            "SELECT id, name, api_keys, endpoints, models, custom_headers, COALESCE(timeout_secs, 300), COALESCE(max_concurrency, 0), extras, COALESCE(failure_threshold, 3), COALESCE(blacklist_minutes, 10) FROM channels WHERE id = ? AND enabled = 1"
        )
        .bind(channel_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        let (
            id,
            name,
            api_keys_str,
            endpoints_str,
            models_str,
            custom_headers_str,
            timeout_secs,
            max_concurrency,
            extras_str,
            failure_threshold,
            blacklist_minutes,
        ) = result.ok_or_else(|| ProxyError::ChannelNotFound("渠道不存在或已禁用".to_string()))?;

        let api_keys: Vec<UpstreamApiKey> = parse_api_keys(&api_keys_str);
        let endpoints: Vec<EndpointConfig> =
            serde_json::from_str(&endpoints_str).unwrap_or_default();
        let models = parse_models(&models_str);
        let custom_headers: Vec<CustomHeader> =
            serde_json::from_str(&custom_headers_str).unwrap_or_default();
        let extras: Option<serde_json::Map<String, serde_json::Value>> =
            serde_json::from_str(&extras_str).ok();

        let channel = ChannelInfo {
            id,
            name,
            api_keys,
            endpoints,
            models,
            custom_headers,
            timeout_secs: timeout_secs as u64,
            max_concurrency: max_concurrency as u32,
            extras,
            failure_threshold: failure_threshold as u64,
            blacklist_minutes: blacklist_minutes as i64,
        };

        // 3. 写入缓存
        self.cache.set_channel(channel.clone()).await;

        Ok(channel)
    }

    /// 生成一次请求内的同渠道 Key 尝试序列（跳过禁用 Key）。
    pub fn api_key_attempts(&self, channel: &ChannelInfo) -> Vec<String> {
        let enabled_keys = channel.enabled_api_keys();
        if enabled_keys.is_empty() {
            return vec![String::new()];
        }

        let start = self.key_counter.fetch_add(1, Ordering::Relaxed) as usize % enabled_keys.len();

        (0..enabled_keys.len())
            .map(|offset| {
                enabled_keys[(start + offset) % enabled_keys.len()]
                    .key
                    .clone()
            })
            .collect()
    }
}

/// 解析 models 字段
fn parse_models(models_str: &str) -> Vec<String> {
    serde_json::from_str(models_str).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_models_handles_valid_json() {
        let models = parse_models(r#"["gpt-4", "gpt-3.5"]"#);
        assert_eq!(models, vec!["gpt-4", "gpt-3.5"]);
    }

    #[test]
    fn parse_models_falls_back_on_invalid_json() {
        let models = parse_models("not json");
        assert!(models.is_empty());
    }

    #[test]
    fn parse_models_falls_back_on_empty_string() {
        let models = parse_models("");
        assert!(models.is_empty());
    }
}
