//! Budget 数据访问层（v1.1.2 从 stats.rs 迁入）。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, SqlitePool};

use crate::domain::budget::SetBudgetRequest;
use crate::util::timeutil::tz_modifier;

/// 预算限制（带 FromRow，留 repository 层）
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct BudgetLimit {
    pub id: String,
    pub api_key_id: String,
    pub monthly_limit_usd: f64,
    pub daily_limit_usd: f64,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[async_trait]
pub trait BudgetRepository: Send + Sync {
    /// 设置/更新预算限制（upsert）。返回 None 表示 api_key 不存在。
    async fn set_budget(&self, req: SetBudgetRequest) -> Result<Option<BudgetLimit>, sqlx::Error>;
    async fn list_budgets(&self) -> Result<Vec<BudgetLimit>, sqlx::Error>;
    /// 删除预算限制，返回是否实际删除。
    async fn delete_budget(&self, id: &str) -> Result<bool, sqlx::Error>;

    /// proxy 预算门：按 api_key_id 取启用的 (月限额, 日限额)，无则 None。
    async fn get_limits(&self, api_key_id: &str) -> Result<Option<(f64, f64)>, sqlx::Error>;
}

pub struct SqliteBudgetRepository {
    pool: SqlitePool,
    timezone_offset: i32,
}

impl SqliteBudgetRepository {
    pub fn new(pool: SqlitePool, timezone_offset: i32) -> Self {
        Self {
            pool,
            timezone_offset,
        }
    }

    fn tz_modifier(&self) -> String {
        tz_modifier(self.timezone_offset)
    }
}

#[async_trait]
impl BudgetRepository for SqliteBudgetRepository {
    async fn set_budget(&self, req: SetBudgetRequest) -> Result<Option<BudgetLimit>, sqlx::Error> {
        let monthly = req.monthly_limit_usd.unwrap_or(0.0);
        let daily = req.daily_limit_usd.unwrap_or(0.0);
        let enabled = req.enabled.unwrap_or(true);

        // 验证 API Key 存在
        let count: i32 = sqlx::query_scalar("SELECT COUNT(*) FROM api_keys WHERE id = ?")
            .bind(&req.api_key_id)
            .fetch_one(&self.pool)
            .await?;
        if count == 0 {
            return Ok(None);
        }

        let id = crate::util::id::generate_id();
        sqlx::query(
            r#"INSERT INTO budget_limits (id, api_key_id, monthly_limit_usd, daily_limit_usd, enabled)
               VALUES (?, ?, ?, ?, ?)
               ON CONFLICT(api_key_id) DO UPDATE SET
                   monthly_limit_usd = excluded.monthly_limit_usd,
                   daily_limit_usd = excluded.daily_limit_usd,
                   enabled = excluded.enabled,
                   updated_at = datetime('now')"#,
        )
        .bind(&id)
        .bind(&req.api_key_id)
        .bind(monthly)
        .bind(daily)
        .bind(enabled)
        .execute(&self.pool)
        .await?;

        // 查询返回
        let tz = self.tz_modifier();
        let row = sqlx::query_as::<_, BudgetLimit>(
            AssertSqlSafe(format!("SELECT id, api_key_id, monthly_limit_usd, daily_limit_usd, enabled, datetime(created_at, '{}') as created_at, datetime(updated_at, '{}') as updated_at FROM budget_limits WHERE api_key_id = ?", tz, tz).as_str()),
        )
        .bind(&req.api_key_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(Some(row))
    }

    async fn list_budgets(&self) -> Result<Vec<BudgetLimit>, sqlx::Error> {
        let tz = self.tz_modifier();
        let rows = sqlx::query_as::<_, BudgetLimit>(
            AssertSqlSafe(format!("SELECT id, api_key_id, monthly_limit_usd, daily_limit_usd, enabled, datetime(created_at, '{}') as created_at, datetime(updated_at, '{}') as updated_at FROM budget_limits ORDER BY created_at DESC", tz, tz).as_str()),
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn delete_budget(&self, id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM budget_limits WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn get_limits(&self, api_key_id: &str) -> Result<Option<(f64, f64)>, sqlx::Error> {
        sqlx::query_as::<_, (f64, f64)>(
            "SELECT monthly_limit_usd, daily_limit_usd FROM budget_limits WHERE api_key_id = ? AND enabled = 1",
        )
        .bind(api_key_id)
        .fetch_optional(&self.pool)
        .await
    }
}
