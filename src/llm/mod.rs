//! LLM 转发引擎聚合层（v1.1.0 骨架）。
//!
//! 对齐 axonhub `llm/` 大聚合模式（见 docs/design-v1-refactor.md §2.3）。
//! 后续批次从顶层 relay/protocol/scheduler 迁入 + 新增 plugin：
//! - `relay/`：转发管道（pipeline / executor / stream_executor / prepare）
//! - `protocol/`：协议转换（inbound / outbound / sse / converter）
//! - `scheduler/`：负载均衡（选渠道）
//! - `plugin/`：请求/响应拦截改写（cch / tracking / cache_key / thinking）
//!
//! Step A 起逐个迁入顶层模块（protocol / scheduler / relay），plugin 待后续。
pub mod protocol;
pub mod scheduler;
