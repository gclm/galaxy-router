//! 业务逻辑层（v1.1.0 骨架）。
//!
//! 无 HTTP 耦合，调 repository trait。后续批次填：
//! - `pricing/`：模型定价（合并 metrics/model.rs + pricing.rs）
//! - `stats/`：统计聚合 + recorder + redaction
//!
//! v1.1.0 仅占位 Services 空壳。

/// 统一持有所有 service（v1.1.0 空壳）。
#[derive(Debug, Clone)]
pub struct Services;

impl Services {
    pub fn new() -> Self {
        Self
    }
}

impl Default for Services {
    fn default() -> Self {
        Self::new()
    }
}
