use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use std::path::Path;

use crate::config::{RuntimeConfig, SchedulerConfig, ScoreWeights, StickySessionConfig};

/// 设置项（数据库行）
#[derive(Debug, sqlx::FromRow)]
#[allow(dead_code)]
struct SettingRow {
    key: String,
    category: String,
    value: String,
    description: Option<String>,
}

/// 设置项（API 返回）
#[derive(Debug, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct SettingItem {
    pub key: String,
    pub category: String,
    pub value: String,
    pub description: Option<String>,
}

/// 数据库连接池
#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    /// 创建数据库连接
    pub async fn new(database_url: &str) -> Result<Self> {
        // 确保数据目录存在
        if let Some(path) = database_url.strip_prefix("sqlite:")
            && let Some(parent) = Path::new(path).parent()
        {
            std::fs::create_dir_all(parent)?;
        }

        // 对于文件数据库，使用 sqlite:{path} 格式
        let pool = SqlitePoolOptions::new()
            .max_connections(10)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .connect(database_url)
            .await?;

        let db = Self { pool };
        db.run_migrations().await?;
        Ok(db)
    }

    /// 运行数据库迁移
    async fn run_migrations(&self) -> Result<()> {
        sqlx::migrate!("src/db/migrations")
            .run(&self.pool)
            .await
            .map_err(|e| anyhow::anyhow!("数据库迁移失败: {e}"))?;
        Ok(())
    }

    /// 获取连接池引用
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// 从数据库加载运行时配置
    #[allow(dead_code)]
    pub async fn load_runtime_config(&self) -> Result<RuntimeConfig> {
        let settings: Vec<(String, String)> = sqlx::query_as("SELECT key, value FROM settings")
            .fetch_all(&self.pool)
            .await?;

        let settings_map: std::collections::HashMap<String, String> =
            settings.into_iter().collect();

        Ok(self.build_runtime_config(&settings_map))
    }

    /// 按分类查询设置
    #[allow(dead_code)]
    pub async fn get_settings_by_category(&self, category: &str) -> Result<Vec<SettingItem>> {
        let settings = sqlx::query_as::<_, SettingRow>(
            "SELECT key, category, value, description FROM settings WHERE category = ? ORDER BY key"
        )
        .bind(category)
        .fetch_all(&self.pool)
        .await?;

        Ok(settings
            .into_iter()
            .map(|r| SettingItem {
                key: r.key,
                category: r.category,
                value: r.value,
                description: r.description,
            })
            .collect())
    }

    /// 获取所有设置
    #[allow(dead_code)]
    pub async fn get_all_settings(&self) -> Result<Vec<SettingItem>> {
        let settings = sqlx::query_as::<_, SettingRow>(
            "SELECT key, category, value, description FROM settings ORDER BY category, key",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(settings
            .into_iter()
            .map(|r| SettingItem {
                key: r.key,
                category: r.category,
                value: r.value,
                description: r.description,
            })
            .collect())
    }

    /// 更新设置
    #[allow(dead_code)]
    pub async fn update_setting(&self, key: &str, value: &str) -> Result<()> {
        sqlx::query("UPDATE settings SET value = ?, updated_at = CURRENT_TIMESTAMP WHERE key = ?")
            .bind(value)
            .bind(key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    #[allow(dead_code)]
    fn build_runtime_config(
        &self,
        settings_map: &std::collections::HashMap<String, String>,
    ) -> RuntimeConfig {
        let top_k = settings_map
            .get("scheduler.top_k")
            .and_then(|v| v.parse().ok())
            .unwrap_or(7);

        let score_weights: ScoreWeights = settings_map
            .get("scheduler.score_weights")
            .and_then(|v| serde_json::from_str(v).ok())
            .unwrap_or_default();

        let sticky_enabled = settings_map
            .get("sticky_session.enabled")
            .map(|v| v == "true")
            .unwrap_or(true);

        let sticky_ttl = settings_map
            .get("sticky_session.ttl_seconds")
            .and_then(|v| v.parse().ok())
            .unwrap_or(3600);

        RuntimeConfig {
            scheduler: SchedulerConfig {
                top_k,
                score_weights,
            },
            sticky_session: StickySessionConfig {
                enabled: sticky_enabled,
                ttl_seconds: sticky_ttl,
            },
        }
    }
}
