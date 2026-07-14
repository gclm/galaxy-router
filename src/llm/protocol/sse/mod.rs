//! SSE 解析（Step D 拆分：事件解析 / 错误提取 / 完成检测三维度）。
//! 通过 `pub use` 重导出保持 `crate::protocol::sse::*` 路径不变。

pub mod error;
pub mod parsing;
pub mod usage;

pub use error::*;
pub use parsing::*;
pub use usage::*;
