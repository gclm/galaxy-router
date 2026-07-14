//! 代理管道（Step D 拆分：前置门禁 access / 执行 proxy / 派发 dispatch）。
//! `pub use` 重导出保持 `crate::relay::pipeline::*` 路径不变。

pub mod access;
pub mod dispatch;
pub mod proxy;

pub(crate) use access::validate_header_value;
pub use dispatch::handle_proxy_request;
pub use proxy::{proxy_request, proxy_stream};
