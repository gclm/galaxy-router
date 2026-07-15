//! 定价 service：model（ModelRegistry 模型元数据 + 定价计算 + 远程拉取）+ refresher（定时刷新）。
//! B2-C3 从 metrics/model.rs + metrics/pricing.rs 迁入。

pub mod model;
pub mod refresher;
