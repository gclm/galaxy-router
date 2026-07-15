use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::{SqliteConnection, SqlitePool};

type DbResult<T> = Result<T, (StatusCode, Json<ApiError>)>;
type RouteRow = (String, String, Option<String>, bool, i32, bool);

use crate::domain::channel::Channel;
use crate::repository::channel_repository::ChannelRow;
use crate::app_state::AppState;
use crate::error::app::{ApiError, ApiResponse};

const BACKUP_FORMAT: &str = "galaxy-router-backup";
const BACKUP_VERSION: i32 = 1;

/// 导出文件格式
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupFile {
    pub format: String,
    pub version: i32,
    pub exported_at: String,
    pub app_version: String,
    pub data: BackupData,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupData {
    pub channels: Vec<Channel>,
    #[serde(alias = "groups")]
    pub routes: Vec<RouteExport>,
    pub api_keys: Vec<ApiKeyExport>,
    pub settings: Vec<SettingExport>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RouteExport {
    pub name: String,
    pub match_regex: Option<String>,
    pub retry_enabled: bool,
    pub first_token_timeout_secs: i32,
    pub enabled: bool,
    pub items: Vec<RouteItemExport>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RouteItemExport {
    pub channel_name: String,
    pub model_name: String,
    pub priority: i32,
    pub weight: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiKeyExport {
    pub name: String,
    pub api_key: String,
    pub enabled: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SettingExport {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Serialize, Default)]
pub struct ImportResult {
    pub channels_imported: u32,
    pub routes_imported: u32,
    pub api_keys_imported: u32,
    pub settings_imported: u32,
    pub errors: Vec<String>,
}

/// 导出全部配置数据
pub async fn export(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<BackupFile>>, (StatusCode, Json<ApiError>)> {
    let pool = &state.pool;

    let channels = fetch_channels(pool).await?;
    let routes = fetch_groups(pool).await?;
    let api_keys = fetch_api_keys(pool).await?;
    let settings = fetch_settings(pool).await?;

    Ok(Json(ApiResponse::success(BackupFile {
        format: BACKUP_FORMAT.to_string(),
        version: BACKUP_VERSION,
        exported_at: chrono::Utc::now().to_rfc3339(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        data: BackupData {
            channels,
            routes,
            api_keys,
            settings,
        },
    })))
}

/// 导入配置数据
pub async fn import(
    State(state): State<AppState>,
    Json(backup): Json<BackupFile>,
) -> Result<Json<ApiResponse<ImportResult>>, (StatusCode, Json<ApiError>)> {
    if backup.format != BACKUP_FORMAT {
        return Err(ApiError::bad_request("无效的备份文件格式"));
    }
    if backup.version != BACKUP_VERSION {
        return Err(ApiError::bad_request(format!(
            "不支持的备份版本: {}",
            backup.version
        )));
    }

    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    let mut result = ImportResult::default();

    for ch in &backup.data.channels {
        match import_channel(&mut tx, ch).await {
            Ok(true) => result.channels_imported += 1,
            Ok(false) => {}
            Err(e) => result.errors.push(format!("渠道 '{}': {}", ch.name, e)),
        }
    }

    for key in &backup.data.api_keys {
        match import_api_key(&mut tx, key).await {
            Ok(true) => result.api_keys_imported += 1,
            Ok(false) => {}
            Err(e) => result.errors.push(format!("API Key '{}': {}", key.name, e)),
        }
    }

    for s in &backup.data.settings {
        match import_setting(&mut tx, s).await {
            Ok(true) => result.settings_imported += 1,
            Ok(false) => {}
            Err(e) => result.errors.push(format!("设置 '{}': {}", s.key, e)),
        }
    }

    for g in &backup.data.routes {
        match import_group(&mut tx, g).await {
            Ok(true) => result.routes_imported += 1,
            Ok(false) => {}
            Err(e) => result.errors.push(format!("分组 '{}': {}", g.name, e)),
        }
    }

    tx.commit()
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::success(result)))
}

/// 重置结果
#[derive(Debug, Serialize)]
pub struct ResetResult {
    pub channels_deleted: u64,
    pub routes_deleted: u64,
    pub api_keys_deleted: u64,
    pub settings_reset: u64,
}

/// 恢复出厂设置（删除渠道、分组、API Key、设置，保留用户和定价）
pub async fn reset(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<ResetResult>>, (StatusCode, Json<ApiError>)> {
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    let routes_deleted = sqlx::query("DELETE FROM route_items")
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?
        .rows_affected();
    let routes_deleted = routes_deleted
        + sqlx::query("DELETE FROM routes")
            .execute(&mut *tx)
            .await
            .map_err(|e| ApiError::internal_error(e.to_string()))?
            .rows_affected();

    let channels_deleted = sqlx::query("DELETE FROM channels")
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?
        .rows_affected();

    let api_keys_deleted = sqlx::query("DELETE FROM api_keys")
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?
        .rows_affected();

    let settings_reset = sqlx::query("DELETE FROM settings")
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?
        .rows_affected();

    tx.commit()
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::success(ResetResult {
        channels_deleted,
        routes_deleted,
        api_keys_deleted,
        settings_reset,
    })))
}

// ── 数据读取 ──

async fn fetch_channels(pool: &SqlitePool) -> DbResult<Vec<Channel>> {
    let rows: Vec<ChannelRow> = sqlx::query_as(
        "SELECT id, name, api_keys, endpoints, models, rate_limit_rpm, rate_limit_tpm, failure_threshold, blacklist_minutes, concurrency, timeout_secs, max_concurrency, enabled, created_at, updated_at FROM channels ORDER BY created_at",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| ApiError::internal_error(e.to_string()))?;

    rows.into_iter()
        .map(crate::repository::channel_repository::row_to_channel)
        .collect::<Result<Vec<_>, _>>()
        .map_err(ApiError::internal_error)
}

async fn fetch_groups(pool: &SqlitePool) -> DbResult<Vec<RouteExport>> {
    let rows: Vec<RouteRow> = sqlx::query_as(
        "SELECT id, name, match_regex, retry_enabled, first_token_timeout_secs, enabled FROM routes ORDER BY created_at",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| ApiError::internal_error(e.to_string()))?;

    let mut routes = Vec::new();
    for (id, name, match_regex, retry_enabled, first_token_timeout_secs, enabled) in
        rows
    {
        let items: Vec<(String, i32, i32, String)> = sqlx::query_as(
            "SELECT gi.model_name, gi.priority, gi.weight, ch.name
             FROM route_items gi JOIN channels ch ON ch.id = gi.channel_id
             WHERE gi.route_id = ? ORDER BY gi.priority",
        )
        .bind(&id)
        .fetch_all(pool)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

        routes.push(RouteExport {
            name,
            match_regex,
            retry_enabled,
            first_token_timeout_secs,
            enabled,
            items: items
                .into_iter()
                .map(
                    |(model_name, priority, weight, channel_name)| RouteItemExport {
                        channel_name,
                        model_name,
                        priority,
                        weight,
                    },
                )
                .collect(),
        });
    }
    Ok(routes)
}

async fn fetch_api_keys(
    pool: &SqlitePool,
) -> Result<Vec<ApiKeyExport>, (StatusCode, Json<ApiError>)> {
    let rows: Vec<(String, String, bool)> =
        sqlx::query_as("SELECT name, api_key, enabled FROM api_keys ORDER BY created_at")
            .fetch_all(pool)
            .await
            .map_err(|e| ApiError::internal_error(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|(name, api_key, enabled)| ApiKeyExport {
            name,
            api_key,
            enabled,
        })
        .collect())
}

async fn fetch_settings(
    pool: &SqlitePool,
) -> Result<Vec<SettingExport>, (StatusCode, Json<ApiError>)> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT key, value FROM settings ORDER BY category, key")
            .fetch_all(pool)
            .await
            .map_err(|e| ApiError::internal_error(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|(key, value)| SettingExport { key, value })
        .collect())
}

// ── 数据导入（返回 Ok(true)=已导入, Ok(false)=已存在跳过）──

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
        r#"INSERT OR IGNORE INTO api_keys (id, name, api_key, enabled)
           VALUES (?, ?, ?, ?)"#,
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

async fn import_group(conn: &mut SqliteConnection, g: &RouteExport) -> Result<bool, String> {
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
