//! 领域模型层（v1.1.0 骨架，v1.1.2 起填充）。
//!
//! 纯数据结构 + 业务规则，零框架依赖（允许 serde）。
//! 子模块随分层重构逐个填入：
//! - `auth`：InitRequest / LoginRequest / ChangePasswordRequest / AuthResponse / UserInfoResponse（v1.1.2）
//! - `setting`：SettingResponse（v1.1.2）
//! - `route`：Route / RouteItem + 请求 DTO（v1.1.2）
//! - `channel`：Channel / Endpoint / UpstreamKey
//! - `api_key`：ApiKey + 请求 DTO（v1.1.2；parse_supported_models 留 handler 供 relay/proxy 复用）
//! - `budget`：SetBudgetRequest（v1.1.2；BudgetLimit 带 FromRow 留 repository）
//! - `usage`：UsageLog / UsageStats

pub mod api_key;
pub mod auth;
pub mod budget;
pub mod route;
pub mod setting;
pub mod usage;
