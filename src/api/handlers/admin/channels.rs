//! 渠道管理 handlers
//!
//! 按职责切分为三个子模块：
//! - `types` — 渠道/端点 DTO 与枚举
//! - `crud`  — 渠道列表/创建/获取/更新/删除
//! - `probe` — 渠道连通性测试（含流式探测）

mod crud;
mod probe;
mod types;

// 公共类型（部分 re-export 当前未被本 crate 引用，但属于稳定公开 API）
#[allow(unused_imports)]
pub use types::{
    Channel, ChannelState, CreateChannelRequest, CustomHeader, EndpointConfig, EndpointType,
    ListChannelsQuery, PaginatedResponse, TestChannelRequest, TestChannelResponse,
    UpdateChannelRequest, UpstreamApiKey,
};

// crate 内可见的类型（供 backup 等模块使用）
pub(crate) use types::ChannelRow;

// CRUD handlers
pub use crud::{create, delete, get, list, parse_api_keys, update};
pub(crate) use crud::row_to_channel;

// Probe handler
pub use probe::test_channel;
