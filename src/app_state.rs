//! 应用统一状态（v1.1.0 骨架，v1.1.2 起接入 handler）。
//!
//! v1.1.2 起各 admin handler 从各自 `*State` 增量切到 `State<AppState>`。
//! 字段随 handler 迁移逐个补入。

use std::sync::Arc;
use std::time::Instant;

use sqlx::SqlitePool;

use crate::auth::JwtService;
use crate::config::AppConfig;
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
}

impl AppState {
    pub fn new(
        pool: SqlitePool,
        config: Arc<AppConfig>,
        start_time: Arc<Instant>,
        jwt_service: JwtService,
    ) -> Self {
        Self {
            pool: pool.clone(),
            config,
            repositories: Repositories::new(pool),
            services: Services::new(),
            start_time,
            jwt_service,
        }
    }
}
