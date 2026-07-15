//! 应用统一状态。
//!
//! v1.1.2：11 个 admin `*State` 全部合并到 `State<AppState>`。
//! D8：proxy 热路径路由/渠道查询（含缓存）下沉 service/routing，AppState 留薄委托。

use std::sync::Arc;
use std::time::Instant;

use axum::http::StatusCode;
use sqlx::SqlitePool;

use crate::service::update_check::UpdateCheckContext;
use crate::api::middleware::ApiKeyCache;
use crate::auth::JwtService;
use crate::infra::config::AppConfig;
use crate::error::proxy::ProxyError;
use crate::service::pricing::model::ModelRegistry;
use crate::service::routing::RoutingService;
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
    /// proxy 热路径：路由/渠道查询（含缓存协调），薄委托暴露给转发链路
    pub routing: RoutingService,
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
        let routing = RoutingService::new(
            repositories.route.clone(),
            repositories.channel.clone(),
            cache.clone(),
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
            routing,
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

/// 路由与渠道查询（D8：薄委托到 service/routing，proxy 调用方零改动）
impl AppState {
    pub(crate) async fn find_route_by_name(
        &self,
        name: &str,
    ) -> Result<Option<RouteInfo>, ProxyError> {
        self.routing.find_route_by_name(name).await
    }

    pub(crate) async fn find_route_by_regex(
        &self,
        model: &str,
    ) -> Result<Option<RouteInfo>, ProxyError> {
        self.routing.find_route_by_regex(model).await
    }

    pub(crate) async fn get_channel(&self, channel_id: &str) -> Result<ChannelInfo, ProxyError> {
        self.routing.get_channel(channel_id).await
    }

    pub(crate) fn api_key_attempts(&self, channel: &ChannelInfo) -> Vec<String> {
        self.routing.api_key_attempts(channel)
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
        let cache = ProxyCache::new();
        let routing = RoutingService::new(
            repositories.route.clone(),
            repositories.channel.clone(),
            cache.clone(),
        );
        Self {
            pool: pool.clone(),
            config,
            repositories,
            start_time: Arc::new(Instant::now()),
            jwt_service: JwtService::new("test", 24),
            cache,
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
            routing,
            proxy_http_client: reqwest::Client::new(),
            plugin_chain: PluginChain::new_empty(),
        }
    }
}
