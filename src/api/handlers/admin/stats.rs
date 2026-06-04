use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use sqlx::AssertSqlSafe;

use crate::api::{ApiError, ApiResponse};
use crate::stats::{StatsState, tz_modifier};

/// 查询参数
#[derive(Debug, Deserialize)]
pub struct StatsQuery {
    pub days: Option<i32>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

impl StatsQuery {
    /// 获取天数参数，限制在 1..=365 范围内
    fn days(&self) -> i32 {
        self.days.unwrap_or(30).clamp(1, 365)
    }
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
            let days = query.days();
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
            let days = query.days();
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
            let days = query.days();
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
    let days = query.days();
    let stats = state
        .stats
        .get_api_key_stats(days)
        .await
        .map_err(|e: sqlx::Error| ApiError::internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::success(serde_json::json!(stats))))
}

/// 获取延迟百分位统计
pub async fn latency(
    State(state): State<StatsApiState>,
    Query(query): Query<StatsQuery>,
) -> Result<Json<ApiResponse<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let days = query.days();
    let (p50, p95, p99) = state
        .stats
        .get_latency_percentiles(days)
        .await
        .map_err(|e: sqlx::Error| ApiError::internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::success(serde_json::json!({
        "p50_latency_ms": p50,
        "p95_latency_ms": p95,
        "p99_latency_ms": p99,
    }))))
}

// === 预算限制管理 ===

/// 预算限制
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct BudgetLimit {
    pub id: String,
    pub api_key_id: String,
    pub monthly_limit_usd: f64,
    pub daily_limit_usd: f64,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// 设置预算限制请求
#[derive(Debug, Deserialize)]
pub struct SetBudgetRequest {
    pub api_key_id: String,
    pub monthly_limit_usd: Option<f64>,
    pub daily_limit_usd: Option<f64>,
    pub enabled: Option<bool>,
}

/// 设置/更新预算限制（upsert）
pub async fn set_budget(
    State(state): State<StatsApiState>,
    Json(req): Json<SetBudgetRequest>,
) -> Result<Json<ApiResponse<BudgetLimit>>, (StatusCode, Json<ApiError>)> {
    let monthly = req.monthly_limit_usd.unwrap_or(0.0);
    let daily = req.daily_limit_usd.unwrap_or(0.0);
    let enabled = req.enabled.unwrap_or(true);

    // 验证 API Key 存在
    let exists: bool = sqlx::query_scalar::<_, i32>(
        "SELECT COUNT(*) FROM api_keys WHERE id = ?",
    )
    .bind(&req.api_key_id)
    .fetch_one(&state.stats.pool)
    .await
    .map_err(|e| ApiError::internal_error(e.to_string()))
    .map(|c| c > 0)?;

    if !exists {
        return Err(ApiError::not_found("API Key 不存在"));
    }

    let id = crate::api::response::generate_id();
    sqlx::query(
        r#"INSERT INTO budget_limits (id, api_key_id, monthly_limit_usd, daily_limit_usd, enabled)
           VALUES (?, ?, ?, ?, ?)
           ON CONFLICT(api_key_id) DO UPDATE SET
               monthly_limit_usd = excluded.monthly_limit_usd,
               daily_limit_usd = excluded.daily_limit_usd,
               enabled = excluded.enabled,
               updated_at = datetime('now')"#,
    )
    .bind(&id)
    .bind(&req.api_key_id)
    .bind(monthly)
    .bind(daily)
    .bind(enabled)
    .execute(&state.stats.pool)
    .await
    .map_err(|e| ApiError::internal_error(e.to_string()))?;

    // 查询返回
    let tz = tz_modifier(state.stats.timezone_offset);
    let row = sqlx::query_as::<_, BudgetLimit>(
        AssertSqlSafe(format!("SELECT id, api_key_id, monthly_limit_usd, daily_limit_usd, enabled, datetime(created_at, '{}') as created_at, datetime(updated_at, '{}') as updated_at FROM budget_limits WHERE api_key_id = ?", tz, tz).as_str()),
    )
    .bind(&req.api_key_id)
    .fetch_one(&state.stats.pool)
    .await
    .map_err(|e| ApiError::internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::success(row)))
}

/// 获取所有预算限制
pub async fn list_budgets(
    State(state): State<StatsApiState>,
) -> Result<Json<ApiResponse<Vec<BudgetLimit>>>, (StatusCode, Json<ApiError>)> {
    let tz = tz_modifier(state.stats.timezone_offset);
    let rows = sqlx::query_as::<_, BudgetLimit>(
        AssertSqlSafe(format!("SELECT id, api_key_id, monthly_limit_usd, daily_limit_usd, enabled, datetime(created_at, '{}') as created_at, datetime(updated_at, '{}') as updated_at FROM budget_limits ORDER BY created_at DESC", tz, tz).as_str()),
    )
    .fetch_all(&state.stats.pool)
    .await
    .map_err(|e| ApiError::internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::success(rows)))
}

/// 删除预算限制
pub async fn delete_budget(
    State(state): State<StatsApiState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<ApiResponse<()>>, (StatusCode, Json<ApiError>)> {
    let result = sqlx::query("DELETE FROM budget_limits WHERE id = ?")
        .bind(&id)
        .execute(&state.stats.pool)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("预算限制不存在"));
    }

    Ok(Json(crate::api::response::success_empty()))
}
