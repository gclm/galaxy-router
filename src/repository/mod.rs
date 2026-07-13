//! 数据访问层（v1.1.0 骨架，v1.1.2 起填入各 repository trait + impl）。
//!
//! SQL 隔离在此层。各 repository 随 handler 迁移逐个填入：
//! - `settings_repository` / `system_info_repository`（v1.1.2）
//! - `auth/usage/budget/route/api_key/channel/backup`（v1.1.2 后续 commit）

use std::sync::Arc;

use sqlx::SqlitePool;

pub mod settings_repository;
pub mod system_info_repository;
use settings_repository::{SettingsRepository, SqliteSettingsRepository};
use system_info_repository::{SqliteSystemInfoRepository, SystemInfoRepository};

/// 统一持有所有 repository（v1.1.2 起随 handler 迁移逐个填入）。
#[derive(Clone)]
pub struct Repositories {
    pub settings: Arc<dyn SettingsRepository>,
    pub system_info: Arc<dyn SystemInfoRepository>,
}

impl Repositories {
    /// 构造 repository 集合（注入 pool 给各 repository）。
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            settings: Arc::new(SqliteSettingsRepository::new(pool.clone())),
            system_info: Arc::new(SqliteSystemInfoRepository::new(pool)),
        }
    }
}
