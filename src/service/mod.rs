//! 业务逻辑层（无 HTTP 耦合，调 repository trait）。
//!
//! 按业务能力组织：`pricing/`（定价）、`stats/`（统计记录与聚合）。
//! 其余业务能力（channel/route/auth/...）随 service 层补建批次迁入。

pub mod stats;
pub mod pricing;
pub mod backup;
pub mod update_check;
pub mod discovery;
pub mod settings;
pub mod auth;
pub mod channel;
pub mod route;
