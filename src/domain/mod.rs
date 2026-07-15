//! 领域模型层。
//!
//! 纯数据结构 + 业务规则，零框架依赖（允许 serde）。
//! v1.1.2 填入 auth / setting / route / api_key / budget / usage（纯 DTO）。
//! - 带 `sqlx::FromRow` 的行类型留各 repository（UsageLogRow / BudgetLimit 等）。
//! - `channel` 跨层基础类型（EndpointType/EndpointConfig/UpstreamApiKey/CustomHeader）已归位（B1-C1）；
//!   Channel 聚合根 + ChannelRow 行类型 + parse_api_keys 待后续 commit。
//! - `parse_supported_models` 留 `api_keys.rs`（relay/proxy 依赖）。

pub mod api_key;
pub mod auth;
pub mod backup;
pub mod budget;
pub mod channel;
pub mod route;
pub mod setting;
pub mod usage;
