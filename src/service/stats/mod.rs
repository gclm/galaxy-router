//! 统计 service：recorder（落库编排）+ redaction（脱敏）。
//! B2-C2 从 metrics/recorder 迁入。

pub mod recorder;
pub mod redaction;
pub mod usage;
