//! 应用统一状态（v1.1.0 骨架）。
//!
//! v1.1.3 起所有 admin handler 从各自 `*State` 切换到 `State<AppState>`。
//! v1.1.0 仅定义类型 + 在 main.rs 装配，不接入任何 handler / create_router。

use std::sync::Arc;

use sqlx::SqlitePool;

use crate::config::AppConfig;
use crate::repository::Repositories;
use crate::service::Services;

/// 应用统一状态。
///
/// v1.1.0 骨架字段：仅无争议全局共享依赖 + Repositories/Services 空壳。
/// v1.1.3 随 handler 迁移补入 cache / http_client / api_key_cache 等。
#[allow(dead_code)] // v1.1.0 骨架：字段将在 v1.1.3 被 handler 消费
#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<AppConfig>,
    pub repositories: Repositories,
    pub services: Services,
}

impl AppState {
    /// 装配应用状态（v1.1.0 骨架：仅注入 pool + config）。
    pub fn new(pool: SqlitePool, config: Arc<AppConfig>) -> Self {
        Self {
            pool,
            config,
            repositories: Repositories::new(),
            services: Services::new(),
        }
    }
}
