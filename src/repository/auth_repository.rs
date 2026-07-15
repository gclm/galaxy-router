//! Auth 数据访问层（v1.1.2）。

use async_trait::async_trait;
use sqlx::SqlitePool;

#[async_trait]
pub trait AuthRepository: Send + Sync {
    /// 初始化系统：插入首个管理员（仅当 users 表为空）+ 可选写入 site.title。
    /// 返回是否实际插入（false 表示系统已初始化）。
    async fn init(
        &self,
        user_id: &str,
        username: &str,
        password_hash: &str,
        site_title: Option<&str>,
    ) -> Result<bool, sqlx::Error>;

    /// 按用户名查找用户，返回 (id, username, password_hash)。
    async fn find_by_username(
        &self,
        username: &str,
    ) -> Result<Option<(String, String, String)>, sqlx::Error>;

    /// 获取用户密码哈希。
    async fn get_password_hash(&self, user_id: &str) -> Result<String, sqlx::Error>;

    /// 更新用户密码哈希，返回是否更新成功。
    async fn update_password(
        &self,
        user_id: &str,
        password_hash: &str,
    ) -> Result<bool, sqlx::Error>;

    /// 用户总数（健康检查 needs_setup 判定）。
    async fn count_users(&self) -> Result<i64, sqlx::Error>;
}

pub struct SqliteAuthRepository {
    pool: SqlitePool,
}

impl SqliteAuthRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AuthRepository for SqliteAuthRepository {
    async fn init(
        &self,
        user_id: &str,
        username: &str,
        password_hash: &str,
        site_title: Option<&str>,
    ) -> Result<bool, sqlx::Error> {
        let mut tx = self.pool.begin().await?;

        // INSERT ... WHERE NOT EXISTS 防止并发竞态创建多个管理员
        let inserted = sqlx::query(
            "INSERT INTO users (id, username, password_hash) SELECT ?, ?, ? WHERE NOT EXISTS (SELECT 1 FROM users)",
        )
        .bind(user_id)
        .bind(username)
        .bind(password_hash)
        .execute(&mut *tx)
        .await?;

        if inserted.rows_affected() == 0 {
            // 未 commit 的事务 drop 时自动回滚
            return Ok(false);
        }

        if let Some(site_title) = site_title {
            sqlx::query(
                "INSERT OR REPLACE INTO settings (key, category, value, description) VALUES ('site.title', 'general', ?, '站点标题')",
            )
            .bind(site_title)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(true)
    }

    async fn find_by_username(
        &self,
        username: &str,
    ) -> Result<Option<(String, String, String)>, sqlx::Error> {
        let user = sqlx::query_as::<_, (String, String, String)>(
            "SELECT id, username, password_hash FROM users WHERE username = ?",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;
        Ok(user)
    }

    async fn get_password_hash(&self, user_id: &str) -> Result<String, sqlx::Error> {
        let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(hash)
    }

    async fn update_password(
        &self,
        user_id: &str,
        password_hash: &str,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            "UPDATE users SET password_hash = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(password_hash)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn count_users(&self) -> Result<i64, sqlx::Error> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&self.pool)
            .await?;
        Ok(count)
    }
}
