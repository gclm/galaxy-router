//! 应用统一状态。
//!
//! v1.1.2：11 个 admin `*State` 全部合并到 `State<AppState>`。
//! v1.1.3 待办：service 层填充（services 字段）、`new` 参数重构为 builder。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use axum::http::StatusCode;
use sqlx::SqlitePool;

use crate::api::handlers::admin::channels::{EndpointConfig, UpstreamApiKey, parse_api_keys};
use crate::api::handlers::admin::update_check::UpdateCheckContext;
use crate::api::middleware::ApiKeyCache;
use crate::auth::JwtService;
use crate::config::AppConfig;
use crate::error::proxy::ProxyError;
use crate::metrics::model::ModelRegistry;
use crate::metrics::recorder::StatsRecorder;
use crate::relay::cache::ProxyCache;
use crate::relay::channel::ChannelInfo;
use crate::relay::queue::RequestQueue;
use crate::relay::ratelimit::RateLimiter;
use crate::repository::Repositories;
use crate::scheduler::selector::{RouteInfo, RouteItemInfo};
use crate::scheduler::state::LoadBalancerState;
use crate::service::Services;

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
    #[allow(dead_code)] // v1.1.0 骨架，待 service 层填充
    pub services: Services,
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
    ) -> Self {
        let timezone_offset = config.server.timezone_offset;
        Self {
            pool: pool.clone(),
            config,
            repositories: Repositories::new(pool.clone(), timezone_offset),
            services: Services::new(),
            start_time,
            jwt_service,
            cache,
            api_key_cache,
            channel_http_client,
            model_registry,
            models_http_client,
            update_check,
            lb_state,
            stats_recorder: StatsRecorder::new(pool),
            rate_limiter,
            queue: None,
            key_counter: Arc::new(AtomicU64::new(0)),
            proxy_http_client,
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

        // 2. 缓存未命中，查询数据库
        let result = sqlx::query_as::<_, (String, String)>(
            "SELECT id, name FROM routes WHERE name = ? AND enabled = 1",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        match result {
            Some((id, name)) => {
                let items = self.get_route_items(&id).await?;
                let group = RouteInfo {
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

    /// 根据正则查找路由
    pub(crate) async fn find_route_by_regex(
        &self,
        model: &str,
    ) -> Result<Option<RouteInfo>, ProxyError> {
        let routes = sqlx::query_as::<_, (String, String, Option<String>)>(
            "SELECT id, name, match_regex FROM routes WHERE enabled = 1 AND match_regex IS NOT NULL",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        for (id, name, match_regex) in routes {
            if let Some(pattern) = match_regex
                && let Some(re) = self.cache.get_compiled_regex(&pattern).await
                && re.is_match(model)
            {
                let items = self.get_route_items(&id).await?;
                return Ok(Some(RouteInfo { id, name, items }));
            }
        }

        Ok(None)
    }

    /// 获取路由项
    async fn get_route_items(&self, route_id: &str) -> Result<Vec<RouteItemInfo>, ProxyError> {
        let items = sqlx::query_as::<_, (String, String, i32, i32)>(
            "SELECT channel_id, model_name, priority, weight FROM route_items WHERE route_id = ? ORDER BY priority ASC, weight DESC",
        )
        .bind(route_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        Ok(items
            .into_iter()
            .map(|(channel_id, model_name, priority, weight)| RouteItemInfo {
                channel_id,
                model_name,
                priority,
                weight,
            })
            .collect())
    }

    /// 获取渠道信息（带缓存）
    pub(crate) async fn get_channel(&self, channel_id: &str) -> Result<ChannelInfo, ProxyError> {
        // 1. 检查缓存
        if let Some(channel) = self.cache.get_channel(channel_id).await {
            return Ok(channel);
        }

        // 2. 缓存未命中，查询数据库
        let result = sqlx::query_as::<_, (String, String, String, String, String, i32, i32, i32, i32)>(
            "SELECT id, name, api_keys, endpoints, models, COALESCE(timeout_secs, 300), COALESCE(max_concurrency, 0), COALESCE(failure_threshold, 3), COALESCE(blacklist_minutes, 10) FROM channels WHERE id = ? AND enabled = 1",
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
            timeout_secs,
            max_concurrency,
            failure_threshold,
            blacklist_minutes,
        ) = result.ok_or_else(|| ProxyError::ChannelNotFound("渠道不存在或已禁用".to_string()))?;

        let api_keys: Vec<UpstreamApiKey> = parse_api_keys(&api_keys_str);
        let endpoints: Vec<EndpointConfig> =
            serde_json::from_str(&endpoints_str).unwrap_or_default();
        let models = parse_models(&models_str);

        let channel = ChannelInfo {
            id,
            name,
            api_keys,
            endpoints,
            models,
            timeout_secs: timeout_secs as u64,
            max_concurrency: max_concurrency as u32,
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
            .map(|offset| enabled_keys[(start + offset) % enabled_keys.len()].key.clone())
            .collect()
    }
}

/// 解析 models 字段（原 state.rs 私有 fn）
fn parse_models(models_str: &str) -> Vec<String> {
    serde_json::from_str(models_str).unwrap_or_default()
}

#[cfg(test)]
impl AppState {
    /// 测试构造：最小参数（替代原 ProxyState::new(pool, model_registry)）。
    /// 转发链路测试只用 pool/lb_state/cache/model_registry/stats_recorder/proxy_http_client 等，
    /// 其余字段填合理默认。
    pub fn new_for_test(pool: SqlitePool, model_registry: ModelRegistry) -> Self {
        use crate::config::{AuthConfig, DatabaseConfig, LoggingConfig, ServerConfig};
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
            queuing: crate::config::QueuingConfig::default(),
            pricing: crate::config::PricingTomlConfig::default(),
        });
        Self {
            pool: pool.clone(),
            config,
            repositories: Repositories::new(pool.clone(), 0),
            services: Services::new(),
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
            stats_recorder: StatsRecorder::new(pool),
            rate_limiter: RateLimiter::new(),
            queue: None,
            key_counter: Arc::new(AtomicU64::new(0)),
            proxy_http_client: reqwest::Client::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_models_handles_valid_json() {
        assert_eq!(
            parse_models(r#"["gpt-4", "gpt-3.5"]"#),
            vec!["gpt-4", "gpt-3.5"]
        );
    }

    #[test]
    fn parse_models_falls_back_on_invalid_json() {
        assert!(parse_models("not json").is_empty());
    }

    #[test]
    fn parse_models_falls_back_on_empty_string() {
        assert!(parse_models("").is_empty());
    }
}
