//! Settings 数据访问层（v1.1.2）。

use async_trait::async_trait;
use sqlx::SqlitePool;

use crate::domain::setting::SettingResponse;

#[async_trait]
pub trait SettingsRepository: Send + Sync {
    async fn list(&self) -> Result<Vec<SettingResponse>, sqlx::Error>;
    /// 读取指定 key 的值
    async fn get(&self, key: &str) -> Result<Option<String>, sqlx::Error>;
    /// 更新指定 key，返回是否存在（rows_affected > 0）
    async fn update(&self, key: &str, value: &str) -> Result<bool, sqlx::Error>;
}

pub struct SqliteSettingsRepository {
    pool: SqlitePool,
}

impl SqliteSettingsRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SettingsRepository for SqliteSettingsRepository {
    async fn list(&self) -> Result<Vec<SettingResponse>, sqlx::Error> {
        let rows: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
            "SELECT key, category, value, description FROM settings ORDER BY category, key",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(key, category, value, description)| SettingResponse {
                key,
                category,
                value,
                description,
            })
            .collect())
    }

    async fn get(&self, key: &str) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
    }

    async fn update(&self, key: &str, value: &str) -> Result<bool, sqlx::Error> {
        let result =
            sqlx::query("UPDATE settings SET value = ?, updated_at = CURRENT_TIMESTAMP WHERE key = ?")
                .bind(value)
                .bind(key)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }
}
