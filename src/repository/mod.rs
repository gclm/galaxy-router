//! 数据访问层。
//!
//! SQL 隔离在此层。v1.1.2 填入 8 个 repository trait + impl：
//! settings / system_info / auth / usage / budget / route / api_key / channel。
//! backup 的 SQL 暂留 handler（跨表组装 + row_to_channel 依赖，留 v1.1.3+）。

use std::sync::Arc;

use sqlx::SqlitePool;

pub mod api_key_repository;
pub mod auth_repository;
pub mod backup_repository;
pub mod budget_repository;
pub mod channel_repository;
pub mod model_info_repository;
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

/// SQLite 约束冲突分类（供 service 层 map_err 用；§6：repository 不定义错误类型）。
///
/// SQLite primary code 19（SQLITE_CONSTRAINT）无法区分 UNIQUE / FOREIGN KEY，
/// 故优先看 extended code，回落 message 嗅探（与原 handler 嗅探文本一致）。
pub enum ConstraintKind {
    UniqueViolation,
    ForeignKeyViolation,
}

pub fn classify_constraint(e: &sqlx::Error) -> Option<ConstraintKind> {
    let sqlx::Error::Database(db) = e else {
        return None;
    };
    match db.code().as_deref() {
        // SQLITE_CONSTRAINT_UNIQUE(2067) / PRIMARY_KEY(1555) extended codes
        Some("2067") | Some("1555") => return Some(ConstraintKind::UniqueViolation),
        // SQLITE_CONSTRAINT_FOREIGNKEY(787)
        Some("787") => return Some(ConstraintKind::ForeignKeyViolation),
        _ => {}
    }
    // primary code（"19"）无法细分，回落 message
    let msg = db.message();
    if msg.contains("UNIQUE constraint failed") {
        Some(ConstraintKind::UniqueViolation)
    } else if msg.contains("FOREIGN KEY constraint failed") {
        Some(ConstraintKind::ForeignKeyViolation)
    } else {
        None
    }
}
