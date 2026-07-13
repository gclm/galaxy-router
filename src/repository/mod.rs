//! 数据访问层（v1.1.0 骨架）。
//!
//! SQL 隔离在此层。v1.1.3 起填入各 repository trait + impl：
//! - channel_repository / route_repository / api_key_repository
//! - usage_repository（拆 1202 行 metrics/query/mod.rs）
//! - budget_repository / settings_repository
//!
//! v1.1.0 仅占位 Repositories 空壳。

/// 统一持有所有 repository（v1.1.0 空壳，v1.1.3 填字段）。
#[derive(Debug, Clone)]
pub struct Repositories;

impl Repositories {
    /// 构造空 repository 集合（v1.1.3 注入 pool + 各 trait 实现）。
    pub fn new() -> Self {
        Self
    }
}

impl Default for Repositories {
    fn default() -> Self {
        Self::new()
    }
}
