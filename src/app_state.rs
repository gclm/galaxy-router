//! 应用统一状态。
//!
//! v1.1.2：11 个 admin `*State` 全部合并到 `State<AppState>`。
//! v1.1.3 待办：service 层填充（services 字段）、`new` 参数重构为 builder。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use axum::http::StatusCode;
use sqlx::SqlitePool;

use crate::api::handlers::admin::update_check::UpdateCheckContext;
use crate::api::middleware::ApiKeyCache;
use crate::auth::JwtService;
use crate::infra::config::AppConfig;
use crate::error::proxy::ProxyError;
use crate::service::pricing::model::ModelRegistry;
use crate::service::stats::recorder::StatsRecorder;
use crate::infra::cache::ProxyCache;
use crate::domain::channel::ChannelInfo;
use crate::llm::relay::queue::RequestQueue;
use crate::llm::relay::ratelimit::RateLimiter;
use crate::repository::Repositories;
use crate::domain::route::RouteInfo;
use crate::llm::scheduler::state::LoadBalancerState;
use crate::llm::plugin::PluginChain;

/// 应用统一状态。
///
/// Step B（relay 重构）：原 ProxyState 字段并入，proxy 链路统一用 `State<AppState>`。
/// `lb_state` / `api_key_cache` / `cache` 单实例共享（修复双实例 bug：后台健康探测
/// 对 proxy 生效 + admin 改 key 即时失效 + 渠道缓存统一）。
#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<AppConfig>,
    pub repositories: Repositories,
    pub start_time: Arc<Instant>,
    pub jwt_service: JwtService,
    pub cache: ProxyCache,
    pub api_key_cache: ApiKeyCache,
    pub channel_http_client: reqwest::Client,
    pub model_registry: ModelRegistry,
    pub models_http_client: reqwest::Client,
    pub update_check: UpdateCheckContext,
    // --- 原 ProxyState 字段（Step B 并入，删 ProxyState）---
    pub lb_state: LoadBalancerState,
    pub stats_recorder: StatsRecorder,
    pub rate_limiter: RateLimiter,
    pub queue: Option<RequestQueue>,
    key_counter: Arc<AtomicU64>,
    /// 上游转发客户端（300s + 可选 proxy.url，原 ProxyState.http_client）
    pub proxy_http_client: reqwest::Client,
    // --- Step C：请求插件链 ---
    pub plugin_chain: PluginChain,
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pool: SqlitePool,
        config: Arc<AppConfig>,
        start_time: Arc<Instant>,
        jwt_service: JwtService,
        cache: ProxyCache,
        api_key_cache: ApiKeyCache,
        channel_http_client: reqwest::Client,
        model_registry: ModelRegistry,
        models_http_client: reqwest::Client,
        update_check: UpdateCheckContext,
        lb_state: LoadBalancerState,
        rate_limiter: RateLimiter,
        proxy_http_client: reqwest::Client,
        plugin_chain: PluginChain,
    ) -> Self {
        let timezone_offset = config.server.timezone_offset;
        let repositories = Repositories::new(pool.clone(), timezone_offset);
        let stats_recorder = StatsRecorder::new(
            repositories.usage.clone(),
            repositories.settings.clone(),
        );
        Self {
            pool: pool.clone(),
            config,
            repositories,
            start_time,
            jwt_service,
            cache,
            api_key_cache,
            channel_http_client,
            model_registry,
            models_http_client,
            update_check,
            lb_state,
            stats_recorder,
            rate_limiter,
            queue: None,
            key_counter: Arc::new(AtomicU64::new(0)),
            proxy_http_client,
            plugin_chain,
        }
    }

    /// 设置请求队列（原 ProxyState::with_queue）
    pub fn with_queue(mut self, max_queue_size: usize, timeout_secs: u64) -> Self {
        self.queue = Some(RequestQueue::new(max_queue_size, timeout_secs));
        self
    }
}

/// 代理成功结果（非流式，原 ProxyState::ProxySuccess）
pub struct ProxySuccess {
    pub status: StatusCode,
    pub body: Vec<u8>,
}

/// 路由与渠道查询（带缓存，原 ProxyState impl，Step B 迁入）
impl AppState {
    /// 根据名称查找路由（带缓存）
    pub(crate) async fn find_route_by_name(
        &self,
        name: &str,
    ) -> Result<Option<RouteInfo>, ProxyError> {
        // 1. 检查缓存
        if let Some(group) = self.cache.get_group(name).await {
            return Ok(Some(group));
        }

        // 2. 缓存未命中，查 repository
        let Some((id, route_name)) = self
            .repositories
            .route
            .find_enabled_by_name(name)
            .await
            .map_err(|e| ProxyError::DatabaseError(e.to_string()))?
        else {
            return Ok(None);
        };
        let items = self
            .repositories
            .route
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
    pub(crate) async fn find_route_by_regex(
        &self,
        model: &str,
    ) -> Result<Option<RouteInfo>, ProxyError> {
        let routes = self
            .repositories
            .route
            .list_enabled_with_regex()
            .await
            .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        for (id, name, match_regex) in routes {
            if let Some(pattern) = match_regex
                && let Some(re) = self.cache.get_compiled_regex(&pattern).await
                && re.is_match(model)
            {
                let items = self
                    .repositories
                    .route
                    .list_route_items_for_proxy(&id)
                    .await
                    .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;
                return Ok(Some(RouteInfo { id, name, items }));
            }
        }

        Ok(None)
    }

    /// 获取渠道信息（带缓存）
    pub(crate) async fn get_channel(&self, channel_id: &str) -> Result<ChannelInfo, ProxyError> {
        // 1. 检查缓存
        if let Some(channel) = self.cache.get_channel(channel_id).await {
            return Ok(channel);
        }

        // 2. 缓存未命中，查 repository
        let Some(channel) = self
            .repositories
            .channel
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

        let start = self.key_counter.fetch_add(1, Ordering::Relaxed) as usize % enabled_keys.len();

        (0..enabled_keys.len())
            .map(|offset| enabled_keys[(start + offset) % enabled_keys.len()].key.clone())
            .collect()
    }
}

#[cfg(test)]
impl AppState {
    /// 测试构造：最小参数（替代原 ProxyState::new(pool, model_registry)）。
    /// 转发链路测试只用 pool/lb_state/cache/model_registry/stats_recorder/proxy_http_client 等，
    /// 其余字段填合理默认。
    pub fn new_for_test(pool: SqlitePool, model_registry: ModelRegistry) -> Self {
        use crate::infra::config::{AuthConfig, DatabaseConfig, LoggingConfig, ServerConfig};
        let config = Arc::new(AppConfig {
            server: ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
                timezone_offset: 0,
            },
            database: DatabaseConfig { path: ":memory:".into() },
            logging: LoggingConfig {
                level: "info".into(),
                format: "compact".into(),
                file: false,
                file_path: String::new(),
                rotation: "daily".into(),
                max_files: 30,
            },
            auth: AuthConfig {
                jwt_secret: "test".into(),
                token_expiry_hours: 24,
            },
            queuing: crate::infra::config::QueuingConfig::default(),
            pricing: crate::infra::config::PricingTomlConfig::default(),
        });
        let repositories = Repositories::new(pool.clone(), 0);
        let stats_recorder = StatsRecorder::new(
            repositories.usage.clone(),
            repositories.settings.clone(),
        );
        Self {
            pool: pool.clone(),
            config,
            repositories,
            start_time: Arc::new(Instant::now()),
            jwt_service: JwtService::new("test", 24),
            cache: ProxyCache::new(),
            api_key_cache: ApiKeyCache::new(),
            channel_http_client: reqwest::Client::new(),
            model_registry,
            models_http_client: reqwest::Client::new(),
            update_check: UpdateCheckContext::with_client(
                reqwest::Client::new(),
                "",
                "test/repo",
                None,
            ),
            lb_state: LoadBalancerState::new(),
            stats_recorder,
            rate_limiter: RateLimiter::new(),
            queue: None,
            key_counter: Arc::new(AtomicU64::new(0)),
            proxy_http_client: reqwest::Client::new(),
            plugin_chain: PluginChain::new_empty(),
        }
    }
}
