//! SystemInfo 数据访问层（v1.1.2）。

use async_trait::async_trait;
use sqlx::SqlitePool;

#[async_trait]
pub trait SystemInfoRepository: Send + Sync {
    /// 返回 (channel_count, route_count, api_key_count)
    async fn count_summary(&self) -> Result<(i64, i64, i64), sqlx::Error>;
}

pub struct SqliteSystemInfoRepository {
    pool: SqlitePool,
}

impl SqliteSystemInfoRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SystemInfoRepository for SqliteSystemInfoRepository {
    async fn count_summary(&self) -> Result<(i64, i64, i64), sqlx::Error> {
        let channel_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM channels")
            .fetch_one(&self.pool)
            .await?;
        let route_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM routes")
            .fetch_one(&self.pool)
            .await?;
        let api_key_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM api_keys")
            .fetch_one(&self.pool)
            .await?;
        Ok((channel_count, route_count, api_key_count))
    }
}
