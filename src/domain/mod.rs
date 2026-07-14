//! 领域模型层。
//!
//! 纯数据结构 + 业务规则，零框架依赖（允许 serde）。
//! v1.1.2 填入 auth / setting / route / api_key / budget / usage（纯 DTO）。
//! - 带 `sqlx::FromRow` 的行类型留各 repository（UsageLogRow / BudgetLimit 等）。
//! - `channel` 未进 domain：Channel / EndpointType 等被 relay/proxy 9+ 处依赖，留 `channels/types`（待 v1.1.4/5 relay 重构后归位）。
//! - `parse_supported_models` 留 `api_keys.rs`（relay/proxy 依赖）。

pub mod api_key;
pub mod auth;
pub mod budget;
pub mod route;
pub mod setting;
pub mod usage;
