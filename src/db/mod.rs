use anyhow::Result;
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use std::path::Path;

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

        let pool = SqlitePoolOptions::new()
            .max_connections(10)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .after_connect(|conn, _meta| {
                Box::pin(async move {
                    // 启用 WAL 模式：读写不互相阻塞
                    sqlx::query("PRAGMA journal_mode=WAL")
                        .execute(&mut *conn)
                        .await?;
                    // 并发写冲突时等待 5 秒而非立即报错
                    sqlx::query("PRAGMA busy_timeout=5000")
                        .execute(&mut *conn)
                        .await?;
                    Ok(())
                })
            })
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
}
