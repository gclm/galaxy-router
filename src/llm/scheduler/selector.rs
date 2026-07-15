use crate::api::handlers::admin::channels::EndpointConfig;

// RouteInfo + RouteItemInfo 已归 domain/route（B3-C0），此处 re-export 保兼容
pub use crate::domain::route::{RouteInfo, RouteItemInfo};
use crate::domain::channel::ChannelInfo;

/// 选择结果
#[derive(Debug)]
pub struct SelectionResult {
    pub channel: ChannelInfo,
    pub target_model: String,
    pub endpoint: EndpointConfig,
    pub route_id: Option<String>,
}
