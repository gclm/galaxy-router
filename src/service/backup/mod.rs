//! 备份/恢复业务：元信息组装 + 调用 BackupRepository。
//!
//! handler 只做 format/version 校验 + 调用本 service；SQL 与事务在 BackupRepository。

use sqlx::SqlitePool;

use crate::domain::backup::{
    BackupData, BackupFile, ImportResult, ResetResult, BACKUP_FORMAT, BACKUP_VERSION,
};
use crate::repository::backup_repository::{BackupRepository, SqliteBackupRepository};

pub struct BackupService {
    repo: SqliteBackupRepository,
}

impl BackupService {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            repo: SqliteBackupRepository::new(pool),
        }
    }

    /// 导出全部配置（组装 BackupFile 元信息）。
    pub async fn export(&self) -> Result<BackupFile, sqlx::Error> {
        let data = self.repo.export_all().await?;
        Ok(BackupFile {
            format: BACKUP_FORMAT.to_string(),
            version: BACKUP_VERSION,
            exported_at: chrono::Utc::now().to_rfc3339(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            data,
        })
    }

    /// 导入配置数据（format/version 已在 handler 校验）。
    pub async fn import(&self, data: &BackupData) -> Result<ImportResult, sqlx::Error> {
        self.repo.import_all(data).await
    }

    /// 恢复出厂设置。
    pub async fn reset(&self) -> Result<ResetResult, sqlx::Error> {
        self.repo.reset_all().await
    }
}
