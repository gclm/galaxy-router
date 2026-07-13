use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::error::app::{ApiError, ApiResponse};

// SettingResponse 已移至 domain::setting（v1.1.2），re-export 保兼容
pub use crate::domain::setting::SettingResponse;

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
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<Vec<SettingResponse>>>, (StatusCode, Json<ApiError>)> {
    let items = state
        .repositories
        .settings
        .list()
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;
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
    "github.repo",
    "update.mirror",
];

pub async fn update(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(body): Json<UpdateSettingRequest>,
) -> Result<Json<ApiResponse<()>>, (StatusCode, Json<ApiError>)> {
    if !ALLOWED_SETTING_KEYS.contains(&key.as_str()) {
        return Err(ApiError::bad_request(format!(
            "不允许更新的设置项: {}",
            key
        )));
    }

    let updated = state
        .repositories
        .settings
        .update(&key, &body.value)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    if !updated {
        return Err(ApiError::not_found(format!("设置项 {} 不存在", key)));
    }

    Ok(Json(ApiResponse::success(())))
}

pub async fn infra(
    State(state): State<AppState>,
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
