use crate::domain::channel::{ChannelInfo, EndpointConfig};

/// 选择结果
#[derive(Debug)]
pub struct SelectionResult {
    pub channel: ChannelInfo,
    pub target_model: String,
    pub endpoint: EndpointConfig,
    pub route_id: Option<String>,
}
