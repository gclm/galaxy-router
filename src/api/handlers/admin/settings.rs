use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::sync::Arc;

use crate::api::{ApiError, ApiResponse};
use crate::config::AppConfig;

#[derive(Clone)]
pub struct SettingsState {
    pub pool: SqlitePool,
    pub config: Arc<AppConfig>,
}

#[derive(Debug, Serialize)]
pub struct SettingResponse {
    pub key: String,
    pub category: String,
    pub value: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSettingRequest {
    pub value: String,
}

#[derive(Debug, Serialize)]
pub struct InfraConfigResponse {
    pub server: ServerInfo,
    pub database: DatabaseInfo,
    pub logging: LoggingInfo,
    pub auth: AuthInfo,
}

#[derive(Debug, Serialize)]
pub struct ServerInfo {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Serialize)]
pub struct DatabaseInfo {
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct LoggingInfo {
    pub level: String,
    pub format: String,
    pub file: bool,
    pub file_path: String,
}

#[derive(Debug, Serialize)]
pub struct AuthInfo {
    pub token_expiry_hours: u64,
}

pub async fn list(
    State(state): State<SettingsState>,
) -> Result<Json<ApiResponse<Vec<SettingResponse>>>, (StatusCode, Json<ApiError>)> {
    let rows: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT key, category, value, description FROM settings ORDER BY category, key",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| ApiError::internal_error(e.to_string()))?;

    let items = rows
        .into_iter()
        .map(|(key, category, value, description)| SettingResponse {
            key,
            category,
            value,
            description,
        })
        .collect();

    Ok(Json(ApiResponse::success(items)))
}

/// 允许通过 API 更新的设置项白名单
const ALLOWED_SETTING_KEYS: &[&str] = &[
    "scheduler.top_k",
    "scheduler.score_weights",
    "sticky_session.enabled",
    "sticky_session.ttl_seconds",
    "proxy.enabled",
    "proxy.url",
    "cors.allow_origins",
];

pub async fn update(
    State(state): State<SettingsState>,
    Path(key): Path<String>,
    Json(body): Json<UpdateSettingRequest>,
) -> Result<Json<ApiResponse<()>>, (StatusCode, Json<ApiError>)> {
    if !ALLOWED_SETTING_KEYS.contains(&key.as_str()) {
        return Err(ApiError::bad_request(format!(
            "不允许更新的设置项: {}",
            key
        )));
    }

    let result =
        sqlx::query("UPDATE settings SET value = ?, updated_at = CURRENT_TIMESTAMP WHERE key = ?")
            .bind(&body.value)
            .bind(&key)
            .execute(&state.pool)
            .await
            .map_err(|e| ApiError::internal_error(e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(ApiError::not_found(format!("设置项 {} 不存在", key)));
    }

    Ok(Json(ApiResponse::success(())))
}

pub async fn infra(
    State(state): State<SettingsState>,
) -> Result<Json<ApiResponse<InfraConfigResponse>>, (StatusCode, Json<ApiError>)> {
    let cfg = &state.config;
    Ok(Json(ApiResponse::success(InfraConfigResponse {
        server: ServerInfo {
            host: cfg.server.host.clone(),
            port: cfg.server.port,
        },
        database: DatabaseInfo {
            path: cfg.database.path.clone(),
        },
        logging: LoggingInfo {
            level: cfg.logging.level.clone(),
            format: cfg.logging.format.clone(),
            file: cfg.logging.file,
            file_path: cfg.logging.file_path.clone(),
        },
        auth: AuthInfo {
            token_expiry_hours: cfg.auth.token_expiry_hours,
        },
    })))
}
