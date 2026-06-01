use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(sqlx::FromRow)]
pub(crate) struct ChannelRow {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) api_keys: String,
    pub(crate) endpoints: String,
    pub(crate) models: String,
    pub(crate) rate_limit_rpm: Option<i32>,
    pub(crate) rate_limit_tpm: Option<i32>,
    pub(crate) failure_threshold: i32,
    pub(crate) blacklist_minutes: i32,
    pub(crate) concurrency: i32,
    pub(crate) custom_headers: String,
    pub(crate) enabled: bool,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

use crate::api::{ApiError, ApiResponse, response::generate_id};

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

/// 端点类型
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EndpointType {
    #[serde(rename = "openai_chat")]
    OpenAiChat,
    #[serde(rename = "openai_response")]
    OpenAiResponse,
    Anthropic,
    Gemini,
    #[serde(rename = "openai_embedding")]
    OpenAiEmbedding,
    #[serde(rename = "openai_images")]
    OpenAiImages,
}

impl EndpointType {
    /// 获取端点路径
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAiChat => "openai_chat",
            Self::OpenAiResponse => "openai_response",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
            Self::OpenAiEmbedding => "openai_embedding",
            Self::OpenAiImages => "openai_images",
        }
    }

    pub fn path(&self) -> &'static str {
        match self {
            Self::OpenAiChat => "/chat/completions",
            Self::OpenAiResponse => "/responses",
            Self::Anthropic => "/messages",
            Self::Gemini => "/models/{model}:generateContent",
            Self::OpenAiEmbedding => "/embeddings",
            Self::OpenAiImages => "/images/generations",
        }
    }
}

/// 端点配置
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EndpointConfig {
    #[serde(rename = "type")]
    pub endpoint_type: EndpointType,
    pub base_url: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// 上游 API Key
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UpstreamApiKey {
    pub key: String,
    #[serde(default)]
    pub note: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// 自定义请求头
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CustomHeader {
    pub key: String,
    pub value: String,
}

/// 渠道
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub api_keys: Vec<UpstreamApiKey>,
    pub endpoints: Vec<EndpointConfig>,
    pub models: Vec<String>,
    pub rate_limit_rpm: Option<i32>,
    pub rate_limit_tpm: Option<i32>,
    pub failure_threshold: i32,
    pub blacklist_minutes: i32,
    pub concurrency: i32,
    pub custom_headers: Vec<CustomHeader>,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
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
    pub custom_headers: Option<Vec<CustomHeader>>,
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
    pub custom_headers: Option<Vec<CustomHeader>>,
    pub enabled: Option<bool>,
}

/// 渠道状态
#[derive(Clone)]
pub struct ChannelState {
    pub pool: SqlitePool,
    pub cache: crate::proxy::ProxyCache,
}

/// 获取渠道列表（支持搜索、筛选、排序、分页）
pub async fn list(
    State(state): State<ChannelState>,
    Query(query): Query<ListChannelsQuery>,
) -> Result<Json<ApiResponse<PaginatedResponse<Channel>>>, (StatusCode, Json<ApiError>)> {
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).clamp(1, 100);
    let offset = (page - 1) * page_size;

    let order_field = match query.sort_by.as_deref() {
        Some("name") => "name",
        _ => "created_at",
    };
    let order_dir = match query.sort_order.as_deref() {
        Some("asc") => "ASC",
        _ => "DESC",
    };

    // 构建 COUNT 查询
    let mut count_builder = sqlx::QueryBuilder::new("SELECT COUNT(*) FROM channels");
    let _has_where = push_where(&mut count_builder, &query);

    let count_row = count_builder
        .build()
        .fetch_one(&state.pool)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;
    let total: i64 = sqlx::Row::get(&count_row, 0);

    // 构建数据查询
    let mut data_builder = sqlx::QueryBuilder::new(
        "SELECT id, name, api_keys, endpoints, models, rate_limit_rpm, rate_limit_tpm, failure_threshold, blacklist_minutes, concurrency, custom_headers, enabled, created_at, updated_at FROM channels",
    );
    push_where(&mut data_builder, &query);
    data_builder.push(format!(" ORDER BY {} {} ", order_field, order_dir));
    data_builder.push(" LIMIT ");
    data_builder.push_bind(page_size);
    data_builder.push(" OFFSET ");
    data_builder.push_bind(offset);

    let rows = data_builder
        .build()
        .fetch_all(&state.pool)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    let items: Vec<Channel> = rows.iter().map(row_to_channel_from_row).collect();

    Ok(Json(ApiResponse::success(PaginatedResponse {
        items,
        total,
    })))
}

fn push_where(builder: &mut sqlx::QueryBuilder<sqlx::Sqlite>, query: &ListChannelsQuery) -> bool {
    let mut has_where = false;

    if let Some(ref search) = query.search
        && !search.is_empty()
    {
        builder.push(" WHERE name LIKE ");
        builder.push_bind(format!("%{}%", search));
        has_where = true;
    }
    if let Some(ref status) = query.status {
        let enabled_val = match status.as_str() {
            "enabled" => Some(true),
            "disabled" => Some(false),
            _ => None,
        };
        if let Some(v) = enabled_val {
            builder.push(if has_where {
                " AND enabled = "
            } else {
                " WHERE enabled = "
            });
            builder.push_bind(v);
            has_where = true;
        }
    }
    has_where
}

/// 创建渠道
pub async fn create(
    State(state): State<ChannelState>,
    Json(req): Json<CreateChannelRequest>,
) -> Result<(StatusCode, Json<ApiResponse<Channel>>), (StatusCode, Json<ApiError>)> {
    // 验证输入
    if req.name.is_empty() {
        return Err(ApiError::bad_request("渠道名称不能为空"));
    }
    if req.api_keys.is_empty() {
        return Err(ApiError::bad_request("至少需要一个 API Key"));
    }
    if req.endpoints.is_empty() {
        return Err(ApiError::bad_request("至少需要一个端点"));
    }

    let id = generate_id();
    let api_keys_json = serde_json::to_string(&req.api_keys)
        .map_err(|e| ApiError::internal_error(e.to_string()))?;
    let endpoints_json = serde_json::to_string(&req.endpoints)
        .map_err(|e| ApiError::internal_error(e.to_string()))?;
    let models_json = serde_json::to_string(&req.models.unwrap_or_default())
        .map_err(|e| ApiError::internal_error(e.to_string()))?;
    let custom_headers_json = serde_json::to_string(&req.custom_headers.unwrap_or_default())
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    // 插入渠道
    sqlx::query(
        r#"
        INSERT INTO channels (id, name, api_keys, endpoints, models, rate_limit_rpm, rate_limit_tpm, failure_threshold, blacklist_minutes, concurrency, custom_headers, enabled)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#
    )
    .bind(&id)
    .bind(&req.name)
    .bind(&api_keys_json)
    .bind(&endpoints_json)
    .bind(&models_json)
    .bind(req.rate_limit_rpm)
    .bind(req.rate_limit_tpm)
    .bind(req.failure_threshold.unwrap_or(3))
    .bind(req.blacklist_minutes.unwrap_or(10))
    .bind(req.concurrency.unwrap_or(10))
    .bind(&custom_headers_json)
    .bind(req.enabled.unwrap_or(true))
    .execute(&state.pool)
    .await
    .map_err(|e| {
        if e.to_string().contains("UNIQUE constraint failed") {
            ApiError::conflict("渠道名称已存在")
        } else {
            ApiError::internal_error(e.to_string())
        }
    })?;

    // 返回创建的渠道
    let channel = get_channel_by_id(&state.pool, &id).await?;
    state.cache.invalidate_all_channels().await;
    Ok((StatusCode::CREATED, Json(ApiResponse::success(channel))))
}

/// 获取单个渠道
pub async fn get(
    State(state): State<ChannelState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<Channel>>, (StatusCode, Json<ApiError>)> {
    let channel = get_channel_by_id(&state.pool, &id).await?;
    Ok(Json(ApiResponse::success(channel)))
}

/// 更新渠道
pub async fn update(
    State(state): State<ChannelState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateChannelRequest>,
) -> Result<Json<ApiResponse<Channel>>, (StatusCode, Json<ApiError>)> {
    // 检查渠道是否存在
    let existing = sqlx::query_scalar::<_, String>("SELECT id FROM channels WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.pool)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    if existing.is_none() {
        return Err(ApiError::not_found("渠道不存在"));
    }

    // 构建更新语句
    let mut builder = sqlx::QueryBuilder::new("UPDATE channels SET ");
    let mut separated = builder.separated(", ");
    let mut has_update = false;

    if let Some(ref name) = req.name {
        separated.push("name = ");
        separated.push_bind_unseparated(name);
        has_update = true;
    }
    if let Some(ref api_keys) = req.api_keys {
        separated.push("api_keys = ");
        separated.push_bind_unseparated(serde_json::to_string(api_keys).unwrap_or_default());
        has_update = true;
    }
    if let Some(ref endpoints) = req.endpoints {
        separated.push("endpoints = ");
        separated.push_bind_unseparated(serde_json::to_string(endpoints).unwrap_or_default());
        has_update = true;
    }
    if let Some(ref models) = req.models {
        separated.push("models = ");
        separated.push_bind_unseparated(serde_json::to_string(models).unwrap_or_default());
        has_update = true;
    }
    if let Some(ref custom_headers) = req.custom_headers {
        separated.push("custom_headers = ");
        separated.push_bind_unseparated(serde_json::to_string(custom_headers).unwrap_or_default());
        has_update = true;
    }
    if let Some(enabled) = req.enabled {
        separated.push("enabled = ");
        separated.push_bind_unseparated(enabled);
        has_update = true;
    }
    if let Some(rate_limit_rpm) = req.rate_limit_rpm {
        separated.push("rate_limit_rpm = ");
        separated.push_bind_unseparated(rate_limit_rpm);
        has_update = true;
    }
    if let Some(rate_limit_tpm) = req.rate_limit_tpm {
        separated.push("rate_limit_tpm = ");
        separated.push_bind_unseparated(rate_limit_tpm);
        has_update = true;
    }
    if let Some(failure_threshold) = req.failure_threshold {
        separated.push("failure_threshold = ");
        separated.push_bind_unseparated(failure_threshold);
        has_update = true;
    }
    if let Some(blacklist_minutes) = req.blacklist_minutes {
        separated.push("blacklist_minutes = ");
        separated.push_bind_unseparated(blacklist_minutes);
        has_update = true;
    }
    if let Some(concurrency) = req.concurrency {
        separated.push("concurrency = ");
        separated.push_bind_unseparated(concurrency);
        has_update = true;
    }

    if !has_update {
        return Err(ApiError::bad_request("没有需要更新的字段"));
    }

    separated.push("updated_at = CURRENT_TIMESTAMP");

    builder.push(" WHERE id = ");
    builder.push_bind(&id);

    builder
        .build()
        .execute(&state.pool)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    // 返回更新后的渠道
    let channel = get_channel_by_id(&state.pool, &id).await?;
    state.cache.invalidate_channel(&id).await;
    Ok(Json(ApiResponse::success(channel)))
}

/// 删除渠道
pub async fn delete(
    State(state): State<ChannelState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, (StatusCode, Json<ApiError>)> {
    let result = sqlx::query("DELETE FROM channels WHERE id = ?")
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("渠道不存在"));
    }

    state.cache.invalidate_channel(&id).await;
    Ok(Json(crate::api::response::success_empty()))
}

/// 根据 ID 获取渠道
async fn get_channel_by_id(
    pool: &SqlitePool,
    id: &str,
) -> Result<Channel, (StatusCode, Json<ApiError>)> {
    let result = sqlx::query_as::<_, ChannelRow>(
        "SELECT id, name, api_keys, endpoints, models, rate_limit_rpm, rate_limit_tpm, failure_threshold, blacklist_minutes, concurrency, custom_headers, enabled, created_at, updated_at FROM channels WHERE id = ?"
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::internal_error(e.to_string()))?;

    let row = result.ok_or_else(|| ApiError::not_found("渠道不存在"))?;
    Ok(row_to_channel(row))
}

/// 兼容旧格式：api_keys 可能是 ["sk-xxx"] 或 [{"key":"sk-xxx","note":"","enabled":true}]
pub fn parse_api_keys(json_str: &str) -> Vec<UpstreamApiKey> {
    let value: serde_json::Value = serde_json::from_str(json_str).unwrap_or_default();
    let Some(arr) = value.as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|v| {
            if let Some(s) = v.as_str() {
                Some(UpstreamApiKey {
                    key: s.to_string(),
                    note: String::new(),
                    enabled: true,
                })
            } else {
                serde_json::from_value(v.clone()).ok()
            }
        })
        .collect()
}

fn row_to_channel(row: ChannelRow) -> Channel {
    Channel {
        id: row.id,
        name: row.name,
        api_keys: parse_api_keys(&row.api_keys),
        endpoints: serde_json::from_str(&row.endpoints).unwrap_or_default(),
        models: serde_json::from_str(&row.models).unwrap_or_default(),
        rate_limit_rpm: row.rate_limit_rpm,
        rate_limit_tpm: row.rate_limit_tpm,
        failure_threshold: row.failure_threshold,
        blacklist_minutes: row.blacklist_minutes,
        concurrency: row.concurrency,
        custom_headers: serde_json::from_str(&row.custom_headers).unwrap_or_default(),
        enabled: row.enabled,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn row_to_channel_from_row(row: &sqlx::sqlite::SqliteRow) -> Channel {
    use sqlx::Row;
    Channel {
        id: row.get("id"),
        name: row.get("name"),
        api_keys: parse_api_keys(&row.get::<String, _>("api_keys")),
        endpoints: serde_json::from_str(&row.get::<String, _>("endpoints")).unwrap_or_default(),
        models: serde_json::from_str(&row.get::<String, _>("models")).unwrap_or_default(),
        rate_limit_rpm: row.get("rate_limit_rpm"),
        rate_limit_tpm: row.get("rate_limit_tpm"),
        failure_threshold: row.get("failure_threshold"),
        blacklist_minutes: row.get("blacklist_minutes"),
        concurrency: row.get("concurrency"),
        custom_headers: serde_json::from_str(&row.get::<String, _>("custom_headers"))
            .unwrap_or_default(),
        enabled: row.get("enabled"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}
