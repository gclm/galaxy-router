//! recorder 已迁 service/stats/recorder（B2-C2），此处 re-export 保 crate::metrics::recorder::* 兼容。
pub use crate::service::stats::recorder::*;
