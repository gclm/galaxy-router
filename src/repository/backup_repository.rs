//! 备份数据访问层：集中 backup 特殊 SQL（跨表 JOIN / INSERT OR IGNORE / bulk delete / 单事务）。
//!
//! backup 的 SQL 跨表且语义特殊（导入幂等、单一事务包全部），不适合分散到各实体 repo，故单列。

use async_trait::async_trait;
use sqlx::{SqliteConnection, SqlitePool};

use crate::domain::backup::{
    ApiKeyExport, BackupData, ImportResult, ResetResult, RouteExport, RouteItemExport, SettingExport,
};
use crate::domain::channel::Channel;
use crate::repository::channel_repository::{ChannelRow, row_to_channel};

type RouteRow = (String, String, Option<String>, bool, i32, bool);

/// String 解码错误 → sqlx::Error（row_to_channel 失败时用）。
fn decode_err(e: String) -> sqlx::Error {
    sqlx::Error::Decode(Box::new(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        e,
    )))
}

#[async_trait]
pub trait BackupRepository: Send + Sync {
    /// 导出全部配置（渠道/路由/Key/设置）。
    async fn export_all(&self) -> Result<BackupData, sqlx::Error>;
    /// 导入配置（单一事务，幂等 INSERT OR IGNORE，逐条累积 errors）。
    async fn import_all(&self, data: &BackupData) -> Result<ImportResult, sqlx::Error>;
    /// 恢复出厂（删渠道/路由/Key/设置，保留用户和定价）。
    async fn reset_all(&self) -> Result<ResetResult, sqlx::Error>;
}

pub struct SqliteBackupRepository {
    pool: SqlitePool,
}

impl SqliteBackupRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl BackupRepository for SqliteBackupRepository {
    async fn export_all(&self) -> Result<BackupData, sqlx::Error> {
        let channels = fetch_channels(&self.pool).await?;
        let routes = fetch_routes(&self.pool).await?;
        let api_keys = fetch_api_keys(&self.pool).await?;
        let settings = fetch_settings(&self.pool).await?;
        Ok(BackupData {
            channels,
            routes,
            api_keys,
            settings,
        })
    }

    async fn import_all(&self, data: &BackupData) -> Result<ImportResult, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let mut result = ImportResult::default();

        for ch in &data.channels {
            match import_channel(&mut tx, ch).await {
                Ok(true) => result.channels_imported += 1,
                Ok(false) => {}
                Err(e) => result.errors.push(format!("渠道 '{}': {}", ch.name, e)),
            }
        }
        for key in &data.api_keys {
            match import_api_key(&mut tx, key).await {
                Ok(true) => result.api_keys_imported += 1,
                Ok(false) => {}
                Err(e) => result.errors.push(format!("API Key '{}': {}", key.name, e)),
            }
        }
        for s in &data.settings {
            match import_setting(&mut tx, s).await {
                Ok(true) => result.settings_imported += 1,
                Ok(false) => {}
                Err(e) => result.errors.push(format!("设置 '{}': {}", s.key, e)),
            }
        }
        for g in &data.routes {
            match import_route(&mut tx, g).await {
                Ok(true) => result.routes_imported += 1,
                Ok(false) => {}
                Err(e) => result.errors.push(format!("分组 '{}': {}", g.name, e)),
            }
        }

        tx.commit().await?;
        Ok(result)
    }

    async fn reset_all(&self) -> Result<ResetResult, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let route_items = sqlx::query("DELETE FROM route_items")
            .execute(&mut *tx)
            .await?
            .rows_affected();
        let routes = sqlx::query("DELETE FROM routes")
            .execute(&mut *tx)
            .await?
            .rows_affected();
        let channels = sqlx::query("DELETE FROM channels")
            .execute(&mut *tx)
            .await?
            .rows_affected();
        let api_keys = sqlx::query("DELETE FROM api_keys")
            .execute(&mut *tx)
            .await?
            .rows_affected();
        let settings = sqlx::query("DELETE FROM settings")
            .execute(&mut *tx)
            .await?
            .rows_affected();
        tx.commit().await?;
        Ok(ResetResult {
            channels_deleted: channels,
            routes_deleted: route_items + routes,
            api_keys_deleted: api_keys,
            settings_reset: settings,
        })
    }
}

// ── 导出读取 ──

async fn fetch_channels(pool: &SqlitePool) -> Result<Vec<Channel>, sqlx::Error> {
    let rows: Vec<ChannelRow> = sqlx::query_as(
        "SELECT id, name, api_keys, endpoints, models, rate_limit_rpm, rate_limit_tpm, failure_threshold, blacklist_minutes, concurrency, timeout_secs, max_concurrency, enabled, created_at, updated_at FROM channels ORDER BY created_at",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(row_to_channel)
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_err)
}

async fn fetch_routes(pool: &SqlitePool) -> Result<Vec<RouteExport>, sqlx::Error> {
    let rows: Vec<RouteRow> = sqlx::query_as(
        "SELECT id, name, match_regex, retry_enabled, first_token_timeout_secs, enabled FROM routes ORDER BY created_at",
    )
    .fetch_all(pool)
    .await?;
    let mut routes = Vec::new();
    for (id, name, match_regex, retry_enabled, first_token_timeout_secs, enabled) in rows {
        let items: Vec<(String, i32, i32, String)> = sqlx::query_as(
            "SELECT gi.model_name, gi.priority, gi.weight, ch.name
             FROM route_items gi JOIN channels ch ON ch.id = gi.channel_id
             WHERE gi.route_id = ? ORDER BY gi.priority",
        )
        .bind(&id)
        .fetch_all(pool)
        .await?;
        routes.push(RouteExport {
            name,
            match_regex,
            retry_enabled,
            first_token_timeout_secs,
            enabled,
            items: items
                .into_iter()
                .map(|(model_name, priority, weight, channel_name)| RouteItemExport {
                    channel_name,
                    model_name,
                    priority,
                    weight,
                })
                .collect(),
        });
    }
    Ok(routes)
}

async fn fetch_api_keys(pool: &SqlitePool) -> Result<Vec<ApiKeyExport>, sqlx::Error> {
    let rows: Vec<(String, String, bool)> =
        sqlx::query_as("SELECT name, api_key, enabled FROM api_keys ORDER BY created_at")
            .fetch_all(pool)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(name, api_key, enabled)| ApiKeyExport {
            name,
            api_key,
            enabled,
        })
        .collect())
}

async fn fetch_settings(pool: &SqlitePool) -> Result<Vec<SettingExport>, sqlx::Error> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT key, value FROM settings ORDER BY category, key")
            .fetch_all(pool)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(key, value)| SettingExport { key, value })
        .collect())
}

// ── 导入（返回 Ok(true)=已导入, Ok(false)=已存在跳过）──

async fn import_channel(conn: &mut SqliteConnection, ch: &Channel) -> Result<bool, String> {
    let id = crate::util::id::generate_id();
    let api_keys = serde_json::to_string(&ch.api_keys).map_err(|e| e.to_string())?;
    let endpoints = serde_json::to_string(&ch.endpoints).map_err(|e| e.to_string())?;
    let models = serde_json::to_string(&ch.models).map_err(|e| e.to_string())?;
    let result = sqlx::query(
        r#"INSERT OR IGNORE INTO channels (id, name, api_keys, endpoints, models, rate_limit_rpm, rate_limit_tpm, failure_threshold, blacklist_minutes, concurrency, enabled)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
    )
    .bind(&id)
    .bind(&ch.name)
    .bind(&api_keys)
    .bind(&endpoints)
    .bind(&models)
    .bind(ch.rate_limit_rpm)
    .bind(ch.rate_limit_tpm)
    .bind(ch.failure_threshold)
    .bind(ch.blacklist_minutes)
    .bind(ch.concurrency)
    .bind(ch.enabled)
    .execute(conn)
    .await
    .map_err(|e| e.to_string())?;
    Ok(result.rows_affected() > 0)
}

async fn import_api_key(conn: &mut SqliteConnection, key: &ApiKeyExport) -> Result<bool, String> {
    let id = crate::util::id::generate_id();
    let result = sqlx::query(
        r#"INSERT OR IGNORE INTO api_keys (id, name, api_key, enabled) VALUES (?, ?, ?, ?)"#,
    )
    .bind(&id)
    .bind(&key.name)
    .bind(&key.api_key)
    .bind(key.enabled)
    .execute(conn)
    .await
    .map_err(|e| e.to_string())?;
    Ok(result.rows_affected() > 0)
}

async fn import_setting(conn: &mut SqliteConnection, s: &SettingExport) -> Result<bool, String> {
    let result =
        sqlx::query("UPDATE settings SET value = ?, updated_at = CURRENT_TIMESTAMP WHERE key = ?")
            .bind(&s.value)
            .bind(&s.key)
            .execute(conn)
            .await
            .map_err(|e| e.to_string())?;
    Ok(result.rows_affected() > 0)
}

async fn import_route(conn: &mut SqliteConnection, g: &RouteExport) -> Result<bool, String> {
    let id = crate::util::id::generate_id();
    let result = sqlx::query(
        r#"INSERT OR IGNORE INTO routes (id, name, match_regex, retry_enabled, first_token_timeout_secs, enabled)
           VALUES (?, ?, ?, ?, ?, ?)"#,
    )
    .bind(&id)
    .bind(&g.name)
    .bind(&g.match_regex)
    .bind(g.retry_enabled)
    .bind(g.first_token_timeout_secs)
    .bind(g.enabled)
    .execute(&mut *conn)
    .await
    .map_err(|e| e.to_string())?;
    if result.rows_affected() == 0 {
        return Ok(false);
    }
    for item in &g.items {
        let channel_id: Option<String> =
            sqlx::query_scalar("SELECT id FROM channels WHERE name = ?")
                .bind(&item.channel_name)
                .fetch_optional(&mut *conn)
                .await
                .map_err(|e| e.to_string())?;
        let Some(channel_id) = channel_id else {
            continue;
        };
        let item_id = crate::util::id::generate_id();
        sqlx::query(
            r#"INSERT OR IGNORE INTO route_items (id, route_id, channel_id, model_name, priority, weight)
               VALUES (?, ?, ?, ?, ?, ?)"#,
        )
        .bind(&item_id)
        .bind(&id)
        .bind(&channel_id)
        .bind(&item.model_name)
        .bind(item.priority)
        .bind(item.weight)
        .execute(&mut *conn)
        .await
        .map_err(|e| e.to_string())?;
    }
    Ok(true)
}
