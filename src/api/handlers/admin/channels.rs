//! 渠道管理 handlers
//!
//! 按职责切分为三个子模块：
//! - `types` — 渠道/端点 DTO 与枚举
//! - `crud`  — 渠道列表/创建/获取/更新/删除
//! - `probe` — 渠道连通性测试（含流式探测）

mod crud;
mod probe;
mod types;

pub use types::{
    CreateChannelRequest, ListChannelsQuery, PaginatedResponse, UpdateChannelRequest,
};

pub use crud::{create, delete, get, list, update};
pub use probe::test_endpoint;
