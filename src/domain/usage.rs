//! Usage 领域模型（v1.1.2 从 metrics/query 抽出）。
//!
//! 纯数据结构，零框架依赖（仅 serde）。
//! 带 `sqlx::FromRow` 的行类型（UsageLogRow/UsageLogDetail）留在 repository 层。

use serde::{Deserialize, Serialize};

/// 统计概览
#[derive(Debug, Serialize, Deserialize)]
pub struct StatsOverview {
    pub total_requests: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cost: f64,
    pub today_requests: i64,
    pub today_input_tokens: i64,
    pub today_output_tokens: i64,
    pub today_cost: f64,
    pub latency_p50: Option<f64>,
    pub latency_p95: Option<f64>,
    pub latency_p99: Option<f64>,
}

/// 按模型统计
#[derive(Debug, Serialize, Deserialize)]
pub struct ModelStats {
    pub model: String,
    pub request_count: i32,
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub total_cost: f64,
}

/// 按渠道统计
#[derive(Debug, Serialize, Deserialize)]
pub struct ChannelStats {
    pub channel_id: String,
    pub channel_name: String,
    pub request_count: i32,
    pub success_count: i32,
    pub failure_count: i32,
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub total_cost: f64,
}

/// 每日统计（按天聚合后返回给前端）
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DailyStats {
    pub date: String,
    pub request_count: i32,
    pub success_count: i32,
    pub failure_count: i32,
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub cache_read_tokens: i32,
    pub cache_creation_tokens: i32,
    pub total_cost: f64,
}

/// 按 API Key 统计
#[derive(Debug, Serialize, Deserialize)]
pub struct ApiKeyStats {
    pub api_key_id: String,
    pub api_key_name: Option<String>,
    pub request_count: i32,
    pub success_count: i32,
    pub failure_count: i32,
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub total_cost: f64,
    pub avg_latency_ms: f64,
}

/// 请求日志筛选条件
pub struct LogsFilter {
    pub offset: u32,
    pub limit: u32,
    pub model: Option<String>,
    pub channel_id: Option<String>,
    pub status: Option<String>,
    pub api_key_id: Option<String>,
}

/// 分页结果
pub struct PagedResult<T> {
    pub items: Vec<T>,
    pub total: i64,
}
