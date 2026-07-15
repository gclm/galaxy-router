//! 备份/恢复 handler（薄：format/version 校验 + 调 BackupService）。
//!
//! 数据契约见 domain::backup；SQL 与事务见 repository::backup_repository。

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;

use crate::app_state::AppState;
use crate::domain::backup::{BackupFile, ImportResult, ResetResult, BACKUP_FORMAT, BACKUP_VERSION};
use crate::error::app::{ApiError, ApiResponse};
use crate::service::backup::BackupService;

/// 导出全部配置数据
pub async fn export(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<BackupFile>>, (StatusCode, Json<ApiError>)> {
    let file = BackupService::new(state.pool.clone())
        .export()
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;
    Ok(Json(ApiResponse::success(file)))
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
    let result = BackupService::new(state.pool.clone())
        .import(&backup.data)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;
    Ok(Json(ApiResponse::success(result)))
}

/// 恢复出厂设置（删除渠道、分组、API Key、设置，保留用户和定价）
pub async fn reset(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<ResetResult>>, (StatusCode, Json<ApiError>)> {
    let result = BackupService::new(state.pool.clone())
        .reset()
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;
    Ok(Json(ApiResponse::success(result)))
}
