use serde::{Deserialize, Serialize};

use crate::domain::channel::{CustomHeader, EndpointConfig, UpstreamApiKey};

/// 列表查询参数
#[derive(Debug, Deserialize)]
pub struct ListChannelsQuery {
    pub search: Option<String>,
    pub status: Option<String>,
    pub sort_by: Option<String>,
    pub sort_order: Option<String>,
    pub page: Option<i32>,
    pub page_size: Option<i32>,
}

/// 分页响应
#[derive(Debug, Serialize)]
pub struct PaginatedResponse<T: Serialize> {
    pub items: Vec<T>,
    pub total: i64,
}

/// 创建渠道请求
#[derive(Debug, Deserialize)]
pub struct CreateChannelRequest {
    pub name: String,
    pub api_keys: Vec<UpstreamApiKey>,
    pub endpoints: Vec<EndpointConfig>,
    pub models: Option<Vec<String>>,
    pub rate_limit_rpm: Option<i32>,
    pub rate_limit_tpm: Option<i32>,
    pub failure_threshold: Option<i32>,
    pub blacklist_minutes: Option<i32>,
    pub concurrency: Option<i32>,
    pub timeout_secs: Option<i32>,
    pub max_concurrency: Option<i32>,
    pub enabled: Option<bool>,
}

/// 更新渠道请求
#[derive(Debug, Deserialize)]
pub struct UpdateChannelRequest {
    pub name: Option<String>,
    pub api_keys: Option<Vec<UpstreamApiKey>>,
    pub endpoints: Option<Vec<EndpointConfig>>,
    pub models: Option<Vec<String>>,
    pub rate_limit_rpm: Option<i32>,
    pub rate_limit_tpm: Option<i32>,
    pub failure_threshold: Option<i32>,
    pub blacklist_minutes: Option<i32>,
    pub concurrency: Option<i32>,
    pub timeout_secs: Option<i32>,
    pub max_concurrency: Option<i32>,
    pub enabled: Option<bool>,
}

/// 端点测试请求（不依赖已保存渠道：base_url/headers/api_key/model 都从请求体传，新增渠道也能测）
#[derive(Debug, Deserialize)]
pub struct TestEndpointRequest {
    pub endpoint_type: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub headers: Vec<CustomHeader>,
    pub user_agent: Option<String>,
}

/// 端点测试响应（连通性 + 思维链诊断）
#[derive(Debug, Serialize)]
pub struct TestEndpointResponse {
    pub success: bool,
    pub latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_to_first_token_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_content: Option<String>,
    /// 思维链内容（reasoning_content / thinking block，与正文分开）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u64>,
    /// 思维链诊断：响应是否含 <think> 标签（只检测不应用）
    pub thinking_detected: bool,
    /// 思维链样本（截断到 200 字符）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_sample: Option<String>,
}
