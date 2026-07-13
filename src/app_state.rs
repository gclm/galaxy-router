//! 应用统一状态（v1.1.0 骨架，v1.1.2 起接入 handler）。
//!
//! v1.1.2 起各 admin handler 从各自 `*State` 增量切到 `State<AppState>`。
//! 字段随 handler 迁移逐个补入。

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
#[allow(dead_code)] // v1.1.2 进行中：字段随 handler 迁移逐个被消费，末 commit 删除
#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<AppConfig>,
    pub repositories: Repositories,
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
    #[allow(clippy::too_many_arguments)] // v1.1.2 过渡：参数随字段增加，Commit 10 收尾时重构（builder/Config struct）
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
