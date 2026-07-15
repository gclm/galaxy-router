//! 路由匹配 service：proxy 热路径的路由/渠道查询（带缓存协调）（D8 归位）。
//!
//! 从 app_state.rs 的 4 个 proxy 读路径迁入。缓存读写协调在此；SQL 走 repository。
//! AppState 保留薄委托方法转发到本 service，proxy 调用方零改动。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::domain::channel::ChannelInfo;
use crate::domain::route::RouteInfo;
use crate::error::proxy::ProxyError;
use crate::infra::cache::ProxyCache;
use crate::repository::channel_repository::ChannelRepository;
use crate::repository::route_repository::RouteRepository;

#[derive(Clone)]
pub struct RoutingService {
    route_repo: Arc<dyn RouteRepository>,
    channel_repo: Arc<dyn ChannelRepository>,
    cache: ProxyCache,
    key_counter: Arc<AtomicU64>,
}

impl RoutingService {
    pub fn new(
        route_repo: Arc<dyn RouteRepository>,
        channel_repo: Arc<dyn ChannelRepository>,
        cache: ProxyCache,
    ) -> Self {
        Self {
            route_repo,
            channel_repo,
            cache,
            key_counter: Arc::new(AtomicU64::new(0)),
        }
    }

    /// 根据名称查找路由（带缓存）
    pub async fn find_route_by_name(
        &self,
        name: &str,
    ) -> Result<Option<RouteInfo>, ProxyError> {
        // 1. 检查缓存
        if let Some(group) = self.cache.get_group(name).await {
            return Ok(Some(group));
        }

        // 2. 缓存未命中，查 repository
        let Some((id, route_name)) = self
            .route_repo
            .find_enabled_by_name(name)
            .await
            .map_err(|e| ProxyError::DatabaseError(e.to_string()))?
        else {
            return Ok(None);
        };
        let items = self
            .route_repo
            .list_route_items_for_proxy(&id)
            .await
            .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;
        let group = RouteInfo {
            id,
            name: route_name,
            items,
        };
        // 3. 写入缓存
        self.cache.set_group(group.clone()).await;
        Ok(Some(group))
    }

    /// 根据正则查找路由
    pub async fn find_route_by_regex(
        &self,
        model: &str,
    ) -> Result<Option<RouteInfo>, ProxyError> {
        let routes = self
            .route_repo
            .list_enabled_with_regex()
            .await
            .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        for (id, name, match_regex) in routes {
            if let Some(pattern) = match_regex
                && let Some(re) = self.cache.get_compiled_regex(&pattern).await
                && re.is_match(model)
            {
                let items = self
                    .route_repo
                    .list_route_items_for_proxy(&id)
                    .await
                    .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;
                return Ok(Some(RouteInfo { id, name, items }));
            }
        }

        Ok(None)
    }

    /// 获取渠道信息（带缓存）
    pub async fn get_channel(&self, channel_id: &str) -> Result<ChannelInfo, ProxyError> {
        // 1. 检查缓存
        if let Some(channel) = self.cache.get_channel(channel_id).await {
            return Ok(channel);
        }

        // 2. 缓存未命中，查 repository
        let Some(channel) = self
            .channel_repo
            .get_enabled_for_proxy(channel_id)
            .await
            .map_err(|e| ProxyError::DatabaseError(e.to_string()))?
        else {
            return Err(ProxyError::ChannelNotFound("渠道不存在或已禁用".to_string()));
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

        let start =
            self.key_counter.fetch_add(1, Ordering::Relaxed) as usize % enabled_keys.len();

        (0..enabled_keys.len())
            .map(|offset| enabled_keys[(start + offset) % enabled_keys.len()].key.clone())
            .collect()
    }
}
