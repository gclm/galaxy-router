//! ApiKey 领域模型（v1.1.2 从 api_keys.rs 抽出）。
//!
//! 纯数据结构。`parse_supported_models` 是 relay/proxy 依赖的工具函数，
//! 保留在 `api::handlers::admin::api_keys` 层（不下沉 domain，避免改 relay import）。

use serde::{Deserialize, Serialize};

/// API Key
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ApiKey {
    pub id: String,
    pub name: String,
    pub api_key: String,
    pub enabled: bool,
    pub supported_models: Option<String>,
    pub rate_limit_rpm: i32,
    pub rate_limit_tpm: i32,
    pub allowed_routes: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// 创建 API Key 请求
#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: String,
    pub supported_models: Option<String>,
    pub rate_limit_rpm: Option<i32>,
    pub rate_limit_tpm: Option<i32>,
    pub allowed_routes: Option<String>,
}

/// 更新 API Key 请求
#[derive(Debug, Deserialize)]
pub struct UpdateApiKeyRequest {
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub supported_models: Option<String>,
    pub rate_limit_rpm: Option<i32>,
    pub rate_limit_tpm: Option<i32>,
    pub allowed_routes: Option<String>,
}

/// 列表查询参数
#[derive(Debug, Deserialize)]
pub struct ListApiKeysQuery {
    pub search: Option<String>,
    pub status: Option<String>,
    pub sort_by: Option<String>,
    pub sort_order: Option<String>,
    pub page: Option<i32>,
    pub page_size: Option<i32>,
}
