//! Budget 领域模型（v1.1.2 从 stats.rs 抽出）。
//!
//! 纯数据结构。带 `sqlx::FromRow` 的 BudgetLimit 留在 repository 层。

use serde::Deserialize;

/// 设置预算限制请求
#[derive(Debug, Deserialize)]
pub struct SetBudgetRequest {
    pub api_key_id: String,
    pub monthly_limit_usd: Option<f64>,
    pub daily_limit_usd: Option<f64>,
    pub enabled: Option<bool>,
}
