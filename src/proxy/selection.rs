use crate::api::handlers::admin::channels::EndpointConfig;

use crate::relay::channel::ChannelInfo;

/// 分组信息
#[derive(Debug, Clone)]
pub struct GroupInfo {
    pub id: String,
    pub name: String,
    pub items: Vec<GroupItemInfo>,
}

/// 分组项信息
#[derive(Debug, Clone)]
pub struct GroupItemInfo {
    pub channel_id: String,
    pub model_name: String,
    pub priority: i32,
    /// DB 字段：group_items.weight，保留供未来加权随机使用
    #[allow(dead_code)]
    pub weight: i32,
}

/// 选择结果
#[derive(Debug)]
pub struct SelectionResult {
    pub channel: ChannelInfo,
    pub target_model: String,
    pub endpoint: EndpointConfig,
    pub group_id: Option<String>,
}
