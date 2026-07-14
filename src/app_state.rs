//! 应用统一状态。
//!
//! v1.1.2：11 个 admin `*State` 全部合并到 `State<AppState>`。
//! v1.1.3 待办：service 层填充（services 字段）、`new` 参数重构为 builder。

use std::sync::Arc;
use std::time::Instant;

use sqlx::SqlitePool;

use crate::api::handlers::admin::update_check::UpdateCheckContext;
use crate::api::middleware::ApiKeyCache;
use crate::auth::JwtService;
use crate::config::AppConfig;
use crate::metrics::model::ModelRegistry;
use crate::relay::cache::ProxyCache;
use crate::repository::Repositories;
use crate::service::Services;

/// 应用统一状态。
#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<AppConfig>,
    pub repositories: Repositories,
    #[allow(dead_code)] // v1.1.0 骨架，待 v1.1.3 service 层填充
    pub services: Services,
    pub start_time: Arc<Instant>,
    pub jwt_service: JwtService,
    pub cache: ProxyCache,
    pub api_key_cache: ApiKeyCache,
    pub channel_http_client: reqwest::Client,
    pub model_registry: ModelRegistry,
    pub models_http_client: reqwest::Client,
    pub update_check: UpdateCheckContext,
}

impl AppState {
    #[allow(clippy::too_many_arguments)] // 待 v1.1.3 重构为 builder/Config struct
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
    ) -> Self {
        let timezone_offset = config.server.timezone_offset;
        Self {
            pool: pool.clone(),
            config,
            repositories: Repositories::new(pool, timezone_offset),
            services: Services::new(),
            start_time,
            jwt_service,
            cache,
            api_key_cache,
            channel_http_client,
            model_registry,
            models_http_client,
            update_check,
        }
    }
}
