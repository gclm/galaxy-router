//! Route 领域模型（v1.1.2 从 routes.rs 抽出）。
//!
//! 纯数据结构（允许 serde）。provider 解析等业务规则留 repository（与 SQL 组装耦合）。

use serde::{Deserialize, Serialize};

/// 分组
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Route {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub match_regex: Option<String>,
    pub retry_enabled: bool,
    pub first_token_timeout_secs: i32,
    pub enabled: bool,
    pub items: Vec<RouteItem>,
    pub created_at: String,
    pub updated_at: String,
}

/// 分组项
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RouteItem {
    pub id: String,
    pub channel_id: String,
    pub model_name: String,
    pub priority: i32,
    pub weight: i32,
}

/// 列表查询参数
#[derive(Debug, Deserialize)]
pub struct ListRoutesQuery {
    pub search: Option<String>,
    pub status: Option<String>,
    pub sort_by: Option<String>,
    pub sort_order: Option<String>,
    pub page: Option<i32>,
    pub page_size: Option<i32>,
}

/// 创建分组请求
#[derive(Debug, Deserialize)]
pub struct CreateRouteRequest {
    pub name: String,
    pub provider: Option<String>,
    pub match_regex: Option<String>,
    pub retry_enabled: Option<bool>,
    pub first_token_timeout_secs: Option<i32>,
    pub enabled: Option<bool>,
    pub items: Vec<CreateRouteItemRequest>,
}

/// 创建分组项请求
#[derive(Debug, Deserialize)]
pub struct CreateRouteItemRequest {
    pub channel_id: String,
    pub model_name: String,
    pub priority: Option<i32>,
    pub weight: Option<i32>,
}

/// 更新分组请求
#[derive(Debug, Deserialize)]
pub struct UpdateRouteRequest {
    pub name: Option<String>,
    pub provider: Option<String>,
    pub match_regex: Option<String>,
    pub retry_enabled: Option<bool>,
    pub first_token_timeout_secs: Option<i32>,
    pub enabled: Option<bool>,
    pub items: Option<Vec<CreateRouteItemRequest>>,
}

/// 添加分组项请求
#[derive(Debug, Deserialize)]
pub struct AddRouteItemRequest {
    pub channel_id: String,
    pub model_name: String,
    pub priority: Option<i32>,
    pub weight: Option<i32>,
}
