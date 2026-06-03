use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use serde::Deserialize;

use crate::api::{ApiError, ApiResponse};
use crate::stats::StatsState;

/// 查询参数
#[derive(Debug, Deserialize)]
pub struct StatsQuery {
    pub days: Option<i32>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

/// 统计 API 状态
#[derive(Clone)]
pub struct StatsApiState {
    pub stats: StatsState,
}

/// 获取统计概览
pub async fn overview(
    State(state): State<StatsApiState>,
) -> Result<Json<ApiResponse<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let overview = state
        .stats
        .get_overview()
        .await
        .map_err(|e: sqlx::Error| ApiError::internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::success(serde_json::json!(overview))))
}

/// 获取按模型统计
pub async fn models(
    State(state): State<StatsApiState>,
    Query(query): Query<StatsQuery>,
) -> Result<Json<ApiResponse<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let stats = match (&query.start_date, &query.end_date) {
        (Some(start), Some(end)) => state.stats.get_model_stats_by_range(start, end).await,
        _ => {
            let days = query.days.unwrap_or(30);
            state.stats.get_model_stats(days).await
        }
    }
    .map_err(|e: sqlx::Error| ApiError::internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::success(serde_json::json!(stats))))
}

/// 获取按渠道统计
pub async fn channels(
    State(state): State<StatsApiState>,
    Query(query): Query<StatsQuery>,
) -> Result<Json<ApiResponse<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let stats = match (&query.start_date, &query.end_date) {
        (Some(start), Some(end)) => state.stats.get_channel_stats_by_range(start, end).await,
        _ => {
            let days = query.days.unwrap_or(30);
            state.stats.get_channel_stats(days).await
        }
    }
    .map_err(|e: sqlx::Error| ApiError::internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::success(serde_json::json!(stats))))
}

/// 获取按天统计
pub async fn daily(
    State(state): State<StatsApiState>,
    Query(query): Query<StatsQuery>,
) -> Result<Json<ApiResponse<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let stats = match (&query.start_date, &query.end_date) {
        (Some(start), Some(end)) => state.stats.get_daily_stats_by_range(start, end).await,
        _ => {
            let days = query.days.unwrap_or(30);
            state.stats.get_daily_stats(days).await
        }
    }
    .map_err(|e: sqlx::Error| ApiError::internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::success(serde_json::json!(stats))))
}

/// 请求日志查询参数
#[derive(Debug, Deserialize)]
pub struct LogsQuery {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub model: Option<String>,
    pub channel_id: Option<String>,
    pub status: Option<String>,
    pub api_key_id: Option<String>,
}

/// 获取请求日志
pub async fn logs(
    State(state): State<StatsApiState>,
    Query(query): Query<LogsQuery>,
) -> Result<Json<ApiResponse<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).min(100);
    let offset = (page - 1) * page_size;

    let result = state
        .stats
        .get_logs(crate::stats::LogsFilter {
            offset,
            limit: page_size,
            model: query.model,
            channel_id: query.channel_id,
            status: query.status,
            api_key_id: query.api_key_id,
        })
        .await
        .map_err(|e: sqlx::Error| ApiError::internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::success(serde_json::json!({
        "items": result.items,
        "total": result.total,
    }))))
}

/// 路径参数：日志 ID
#[derive(Debug, Deserialize)]
pub struct LogIdParam {
    pub id: String,
}

/// 获取单条日志详情（含请求/响应内容）
pub async fn log_detail(
    State(state): State<StatsApiState>,
    axum::extract::Path(param): axum::extract::Path<LogIdParam>,
) -> Result<Json<ApiResponse<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let detail = state
        .stats
        .get_log_detail(&param.id)
        .await
        .map_err(|e: sqlx::Error| ApiError::internal_error(e.to_string()))?;

    match detail {
        Some(row) => Ok(Json(ApiResponse::success(serde_json::json!(row)))),
        None => Err(ApiError::not_found("日志不存在")),
    }
}

/// 获取日志中不重复的模型列表（供前端筛选下拉框使用）
pub async fn log_models(
    State(state): State<StatsApiState>,
) -> Result<Json<ApiResponse<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let models = state
        .stats
        .get_log_models()
        .await
        .map_err(|e: sqlx::Error| ApiError::internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::success(serde_json::json!(models))))
}

/// 获取按 API Key 聚合统计
pub async fn api_keys(
    State(state): State<StatsApiState>,
    Query(query): Query<StatsQuery>,
) -> Result<Json<ApiResponse<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let days = query.days.unwrap_or(30);
    let stats = state
        .stats
        .get_api_key_stats(days)
        .await
        .map_err(|e: sqlx::Error| ApiError::internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::success(serde_json::json!(stats))))
}
