//! 数据访问层（v1.1.0 骨架，v1.1.2 起填入各 repository trait + impl）。
//!
//! SQL 隔离在此层。各 repository 随 handler 迁移逐个填入：
//! - `settings_repository` / `system_info_repository`（v1.1.2）
//! - `auth/usage/budget/route/api_key/channel/backup`（v1.1.2 后续 commit）

use std::sync::Arc;

use sqlx::SqlitePool;

pub mod api_key_repository;
pub mod auth_repository;
pub mod budget_repository;
pub mod channel_repository;
pub mod route_repository;
pub mod settings_repository;
pub mod system_info_repository;
pub mod usage_repository;
use api_key_repository::{ApiKeyRepository, SqliteApiKeyRepository};
use auth_repository::{AuthRepository, SqliteAuthRepository};
use budget_repository::{BudgetRepository, SqliteBudgetRepository};
use channel_repository::{ChannelRepository, SqliteChannelRepository};
use route_repository::{RouteRepository, SqliteRouteRepository};
use settings_repository::{SettingsRepository, SqliteSettingsRepository};
use system_info_repository::{SqliteSystemInfoRepository, SystemInfoRepository};
use usage_repository::{SqliteUsageRepository, UsageRepository};

/// 统一持有所有 repository（v1.1.2 起随 handler 迁移逐个填入）。
#[derive(Clone)]
pub struct Repositories {
    pub settings: Arc<dyn SettingsRepository>,
    pub system_info: Arc<dyn SystemInfoRepository>,
    pub auth: Arc<dyn AuthRepository>,
    pub usage: Arc<dyn UsageRepository>,
    pub budget: Arc<dyn BudgetRepository>,
    pub route: Arc<dyn RouteRepository>,
    pub api_key: Arc<dyn ApiKeyRepository>,
    pub channel: Arc<dyn ChannelRepository>,
}

impl Repositories {
    /// 构造 repository 集合（注入 pool + 时区偏移给各 repository）。
    pub fn new(pool: SqlitePool, timezone_offset: i32) -> Self {
        Self {
            settings: Arc::new(SqliteSettingsRepository::new(pool.clone())),
            system_info: Arc::new(SqliteSystemInfoRepository::new(pool.clone())),
            auth: Arc::new(SqliteAuthRepository::new(pool.clone())),
            usage: Arc::new(SqliteUsageRepository::new(pool.clone(), timezone_offset)),
            budget: Arc::new(SqliteBudgetRepository::new(pool.clone(), timezone_offset)),
            route: Arc::new(SqliteRouteRepository::new(pool.clone(), timezone_offset)),
            api_key: Arc::new(SqliteApiKeyRepository::new(pool.clone(), timezone_offset)),
            channel: Arc::new(SqliteChannelRepository::new(pool, timezone_offset)),
        }
    }
}
