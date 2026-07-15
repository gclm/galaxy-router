use serde::{Deserialize, Serialize};

use crate::domain::channel::CustomHeader;

/// 分页响应
#[derive(Debug, Serialize)]
pub struct PaginatedResponse<T: Serialize> {
    pub items: Vec<T>,
    pub total: i64,
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
