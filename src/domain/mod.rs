//! 领域模型层（v1.1.0 骨架）。
//!
//! 纯数据结构 + 业务规则，零框架依赖（允许 serde）。
//! 实际子模块将在后续批次填充：
//! - `channel`：Channel / Endpoint / UpstreamKey
//! - `route`：Route / RouteItem（原 group / group_item，v1.1.2 改名）
//! - `api_key`：ApiKey / BudgetLimit
//! - `usage`：UsageLog / UsageStats / AttemptStats
//! - `proxy`：ProxyRequest 等纯入站数据
//!
//! v1.1.0 不声明任何子模块（避免空文件），仅占位。
