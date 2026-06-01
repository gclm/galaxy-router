use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};

use reqwest::Client;
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
    pub http_client: Client,
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

    let items: Vec<Channel> = rows
        .iter()
        .map(row_to_channel_from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(ApiError::internal_error)?;

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
    Json(mut req): Json<UpdateChannelRequest>,
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
    if let Some(ref mut api_keys) = req.api_keys {
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
    row_to_channel(row).map_err(ApiError::internal_error)
}

pub fn parse_api_keys(json_str: &str) -> Vec<UpstreamApiKey> {
    serde_json::from_str(json_str).unwrap_or_default()
}

fn decode_json_field<T>(field_name: &str, value: &str) -> Result<T, String>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_str(value).map_err(|e| format!("解析 {} 失败: {}", field_name, e))
}

pub(crate) fn row_to_channel(row: ChannelRow) -> Result<Channel, String> {
    Ok(Channel {
        id: row.id,
        name: row.name,
        api_keys: decode_json_field("channels.api_keys", &row.api_keys)?,
        endpoints: decode_json_field("channels.endpoints", &row.endpoints)?,
        models: decode_json_field("channels.models", &row.models)?,
        rate_limit_rpm: row.rate_limit_rpm,
        rate_limit_tpm: row.rate_limit_tpm,
        failure_threshold: row.failure_threshold,
        blacklist_minutes: row.blacklist_minutes,
        concurrency: row.concurrency,
        custom_headers: decode_json_field("channels.custom_headers", &row.custom_headers)?,
        enabled: row.enabled,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

fn row_to_channel_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Channel, String> {
    use sqlx::Row;
    Ok(Channel {
        id: row.get("id"),
        name: row.get("name"),
        api_keys: decode_json_field("channels.api_keys", &row.get::<String, _>("api_keys"))?,
        endpoints: decode_json_field("channels.endpoints", &row.get::<String, _>("endpoints"))?,
        models: decode_json_field("channels.models", &row.get::<String, _>("models"))?,
        rate_limit_rpm: row.get("rate_limit_rpm"),
        rate_limit_tpm: row.get("rate_limit_tpm"),
        failure_threshold: row.get("failure_threshold"),
        blacklist_minutes: row.get("blacklist_minutes"),
        concurrency: row.get("concurrency"),
        custom_headers: decode_json_field(
            "channels.custom_headers",
            &row.get::<String, _>("custom_headers"),
        )?,
        enabled: row.get("enabled"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

// ==================== 渠道测试 ====================

const TEST_PROMPT: &str = "Hello! Please respond with a brief greeting in one sentence.";

/// 测试渠道请求
#[derive(Debug, Deserialize)]
pub struct TestChannelRequest {
    pub model: String,
    pub test_protocol: String,
    pub api_key: String,
    pub stream: Option<bool>,
}

/// 测试渠道响应
#[derive(Debug, Serialize)]
pub struct TestChannelResponse {
    pub success: bool,
    pub message: String,
    pub latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_to_first_token_ms: Option<u64>,
    pub input_prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u64>,
}

/// 构建测试请求体和上游路径
fn build_test_payload(
    protocol: &EndpointType,
    model: &str,
) -> Option<(serde_json::Value, &'static str)> {
    match protocol {
        EndpointType::OpenAiChat => Some((
            serde_json::json!({
                "model": model,
                "messages": [{"role": "user", "content": TEST_PROMPT}],
                "max_tokens": 100,
                "stream": false
            }),
            "/chat/completions",
        )),
        EndpointType::OpenAiResponse => Some((
            serde_json::json!({
                "model": model,
                "input": TEST_PROMPT,
                "max_output_tokens": 100
            }),
            "/responses",
        )),
        EndpointType::Anthropic => Some((
            serde_json::json!({
                "model": model,
                "messages": [{"role": "user", "content": TEST_PROMPT}],
                "max_tokens": 100
            }),
            "/messages",
        )),
        EndpointType::OpenAiEmbedding => Some((
            serde_json::json!({
                "model": model,
                "input": TEST_PROMPT
            }),
            "/embeddings",
        )),
        EndpointType::OpenAiImages => Some((
            serde_json::json!({
                "model": model,
                "prompt": TEST_PROMPT,
                "n": 1,
                "size": "256x256"
            }),
            "/images/generations",
        )),
        _ => None,
    }
}

/// 构建流式测试请求体
fn build_streaming_test_payload(
    protocol: &EndpointType,
    model: &str,
) -> Option<(serde_json::Value, &'static str)> {
    match protocol {
        EndpointType::OpenAiChat => Some((
            serde_json::json!({
                "model": model,
                "messages": [{"role": "user", "content": TEST_PROMPT}],
                "max_tokens": 100,
                "stream": true
            }),
            "/chat/completions",
        )),
        EndpointType::Anthropic => Some((
            serde_json::json!({
                "model": model,
                "messages": [{"role": "user", "content": TEST_PROMPT}],
                "max_tokens": 100,
                "stream": true
            }),
            "/messages",
        )),
        EndpointType::OpenAiResponse => Some((
            serde_json::json!({
                "model": model,
                "input": TEST_PROMPT,
                "max_output_tokens": 100,
                "stream": true
            }),
            "/responses",
        )),
        _ => None,
    }
}

/// 从响应中提取内容文本
fn extract_test_content(resp_body: &serde_json::Value, endpoint_type: &EndpointType) -> String {
    match endpoint_type {
        EndpointType::OpenAiChat => resp_body["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("(无内容)")
            .to_string(),
        EndpointType::OpenAiResponse => resp_body["output"][0]["content"][0]["text"]
            .as_str()
            .unwrap_or("(无内容)")
            .to_string(),
        EndpointType::Anthropic => resp_body["content"][0]["text"]
            .as_str()
            .unwrap_or("(无内容)")
            .to_string(),
        EndpointType::OpenAiEmbedding => {
            let len = resp_body["data"].as_array().map(|a| a.len()).unwrap_or(0);
            format!("Embedding 返回 {} 条向量数据", len)
        }
        EndpointType::OpenAiImages => {
            let count = resp_body["data"].as_array().map(|a| a.len()).unwrap_or(0);
            format!("图片生成成功，共 {} 张", count)
        }
        _ => "(未知协议)".to_string(),
    }
}

/// 从响应中提取 token 用量
fn extract_usage(
    resp_body: &serde_json::Value,
    endpoint_type: &EndpointType,
) -> (Option<u64>, Option<u64>) {
    let usage = &resp_body["usage"];
    match endpoint_type {
        EndpointType::OpenAiChat => (
            usage["prompt_tokens"].as_u64(),
            usage["completion_tokens"].as_u64(),
        ),
        EndpointType::OpenAiResponse | EndpointType::Anthropic => (
            usage["input_tokens"].as_u64(),
            usage["output_tokens"].as_u64(),
        ),
        _ => (None, None),
    }
}

/// 解析协议字符串为 EndpointType
fn parse_protocol(protocol: &str) -> Option<EndpointType> {
    serde_json::from_value::<EndpointType>(serde_json::Value::String(protocol.to_string())).ok()
}

/// 注入自定义请求头
fn inject_custom_headers(
    req_builder: reqwest::RequestBuilder,
    headers: &[CustomHeader],
) -> reqwest::RequestBuilder {
    let mut builder = req_builder;
    for header in headers {
        if let Ok(name) = reqwest::header::HeaderName::from_bytes(header.key.as_bytes())
            && let Ok(value) = header.value.parse::<reqwest::header::HeaderValue>()
        {
            builder = builder.header(name, value);
        }
    }
    builder
}

/// 流式测试：发 SSE 请求，消费完整流，返回首 token 时间和完整内容
async fn send_streaming_test(
    client: &Client,
    url: &str,
    body: &serde_json::Value,
    endpoint_type: &EndpointType,
    api_key: &str,
    custom_headers: &[CustomHeader],
) -> (
    Result<String, String>,
    u64,
    Option<u64>,
    Option<u64>,
    Option<u64>,
) {
    let mut req_builder = client
        .post(url)
        .header("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(60));

    match endpoint_type {
        EndpointType::Anthropic => {
            req_builder = req_builder
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01");
        }
        _ => {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", api_key));
        }
    }
    req_builder = inject_custom_headers(req_builder, custom_headers);

    let start = std::time::Instant::now();
    let resp = match req_builder.json(body).send().await {
        Ok(r) => r,
        Err(e) => {
            return (
                Err(format!("请求上游失败: {}", e)),
                start.elapsed().as_millis() as u64,
                None,
                None,
                None,
            );
        }
    };

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return (
            Err(format!("上游返回 HTTP {}: {}", status, text)),
            start.elapsed().as_millis() as u64,
            None,
            None,
            None,
        );
    }

    let bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => {
            return (
                Err(format!("读取响应失败: {}", e)),
                start.elapsed().as_millis() as u64,
                None,
                None,
                None,
            );
        }
    };

    let text = String::from_utf8_lossy(&bytes);
    let mut first_token_ms: Option<u64> = None;
    let mut full_content = String::new();
    let mut prompt_tokens: Option<u64> = None;
    let mut completion_tokens: Option<u64> = None;

    for line in text.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            let data = data.trim();
            if data == "[DONE]" {
                continue;
            }
            if first_token_ms.is_none() {
                first_token_ms = Some(start.elapsed().as_millis() as u64);
            }
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                if let Some(v) = json["usage"]["prompt_tokens"].as_u64() {
                    prompt_tokens = Some(v);
                }
                if let Some(v) = json["usage"]["completion_tokens"].as_u64() {
                    completion_tokens = Some(v);
                }
                if let Some(v) = json["usage"]["input_tokens"].as_u64() {
                    prompt_tokens = Some(v);
                }
                if let Some(v) = json["usage"]["output_tokens"].as_u64() {
                    completion_tokens = Some(v);
                }
                if let Some(v) = json["message"]["usage"]["input_tokens"].as_u64() {
                    prompt_tokens = Some(v);
                }
                let delta = match endpoint_type {
                    EndpointType::OpenAiChat => json["choices"][0]["delta"]["content"].as_str(),
                    EndpointType::Anthropic => json["delta"]["text"].as_str(),
                    _ => json["delta"]
                        .as_str()
                        .or_else(|| json["choices"][0]["delta"]["content"].as_str()),
                };
                if let Some(d) = delta {
                    full_content.push_str(d);
                }
            }
        }
    }

    let total_ms = start.elapsed().as_millis() as u64;
    if full_content.is_empty() {
        (
            Err("流式响应无内容".to_string()),
            total_ms,
            first_token_ms,
            prompt_tokens,
            completion_tokens,
        )
    } else {
        (
            Ok(full_content),
            total_ms,
            first_token_ms,
            prompt_tokens,
            completion_tokens,
        )
    }
}

/// 测试渠道 — 直接发送到渠道上游，验证指定 key 能否访问指定模型
pub async fn test_channel(
    State(state): State<ChannelState>,
    Path(id): Path<String>,
    Json(req): Json<TestChannelRequest>,
) -> Result<Json<ApiResponse<TestChannelResponse>>, (StatusCode, Json<ApiError>)> {
    let endpoint_type = parse_protocol(&req.test_protocol)
        .ok_or_else(|| ApiError::bad_request(format!("不支持的测试协议: {}", req.test_protocol)))?;

    let use_stream = req.stream.unwrap_or(false);

    // 流式模式下选择流式请求体
    let (body, upstream_path) = if use_stream {
        build_streaming_test_payload(&endpoint_type, &req.model)
    } else {
        build_test_payload(&endpoint_type, &req.model)
    }
    .ok_or_else(|| {
        ApiError::bad_request(format!(
            "协议 {} 不支持{}测试",
            req.test_protocol,
            if use_stream { "流式" } else { "" }
        ))
    })?;

    // 查渠道
    let channel = get_channel_by_id(&state.pool, &id).await?;

    // 按真实 key 查找，渠道测试用于验证指定 key 能否请求指定端点。
    let api_key = channel
        .api_keys
        .iter()
        .find(|k| k.key == req.api_key && k.enabled)
        .ok_or_else(|| ApiError::bad_request("指定的 API Key 不存在或已禁用"))?;

    // 按 test_protocol 查找 endpoint
    let endpoint = channel
        .endpoints
        .iter()
        .find(|e| e.endpoint_type == endpoint_type && e.enabled)
        .ok_or_else(|| ApiError::bad_request(format!("渠道没有启用 {} 端点", req.test_protocol)))?;

    let url = format!(
        "{}{}",
        endpoint.base_url.trim_end_matches('/'),
        upstream_path
    );

    if use_stream {
        let (result, latency_ms, ttft, pt, ct) = send_streaming_test(
            &state.http_client,
            &url,
            &body,
            &endpoint_type,
            &api_key.key,
            &channel.custom_headers,
        )
        .await;

        match result {
            Ok(content) => Ok(Json(ApiResponse::success(TestChannelResponse {
                success: true,
                message: "模型测试成功（流式）".to_string(),
                latency_ms,
                time_to_first_token_ms: ttft,
                input_prompt: TEST_PROMPT.to_string(),
                output_content: Some(content),
                prompt_tokens: pt,
                completion_tokens: ct,
            }))),
            Err(msg) => Ok(Json(ApiResponse::success(TestChannelResponse {
                success: false,
                message: msg,
                latency_ms,
                time_to_first_token_ms: ttft,
                input_prompt: TEST_PROMPT.to_string(),
                output_content: None,
                prompt_tokens: pt,
                completion_tokens: ct,
            }))),
        }
    } else {
        let start = std::time::Instant::now();

        let mut req_builder = state
            .http_client
            .post(&url)
            .header("Content-Type", "application/json")
            .timeout(std::time::Duration::from_secs(30));

        match endpoint_type {
            EndpointType::Anthropic => {
                req_builder = req_builder
                    .header("x-api-key", api_key.key.as_str())
                    .header("anthropic-version", "2023-06-01");
            }
            _ => {
                req_builder =
                    req_builder.header("Authorization", format!("Bearer {}", api_key.key));
            }
        }

        req_builder = inject_custom_headers(req_builder, &channel.custom_headers);

        let resp = req_builder.json(&body).send().await;
        let latency_ms = start.elapsed().as_millis() as u64;

        match resp {
            Ok(resp) => {
                let status = resp.status();
                let resp_text = resp.text().await.unwrap_or_default();

                if !status.is_success() {
                    return Ok(Json(ApiResponse::success(TestChannelResponse {
                        success: false,
                        message: format!("上游返回 HTTP {}: {}", status, resp_text),
                        latency_ms,
                        time_to_first_token_ms: None,
                        input_prompt: TEST_PROMPT.to_string(),
                        output_content: None,
                        prompt_tokens: None,
                        completion_tokens: None,
                    })));
                }

                let resp_body: serde_json::Value =
                    serde_json::from_str(&resp_text).unwrap_or_default();
                if resp_body.get("error").is_some() {
                    let error_msg = resp_body["error"]["message"].as_str().unwrap_or("未知错误");
                    return Ok(Json(ApiResponse::success(TestChannelResponse {
                        success: false,
                        message: format!("模型返回错误: {}", error_msg),
                        latency_ms,
                        time_to_first_token_ms: None,
                        input_prompt: TEST_PROMPT.to_string(),
                        output_content: None,
                        prompt_tokens: None,
                        completion_tokens: None,
                    })));
                }

                let content = extract_test_content(&resp_body, &endpoint_type);
                let (pt, ct) = extract_usage(&resp_body, &endpoint_type);
                Ok(Json(ApiResponse::success(TestChannelResponse {
                    success: true,
                    message: "模型测试成功".to_string(),
                    latency_ms,
                    time_to_first_token_ms: None,
                    input_prompt: TEST_PROMPT.to_string(),
                    output_content: Some(content),
                    prompt_tokens: pt,
                    completion_tokens: ct,
                })))
            }
            Err(e) => Ok(Json(ApiResponse::success(TestChannelResponse {
                success: false,
                message: format!("请求上游失败: {}", e),
                latency_ms,
                time_to_first_token_ms: None,
                input_prompt: TEST_PROMPT.to_string(),
                output_content: None,
                prompt_tokens: None,
                completion_tokens: None,
            }))),
        }
    }
}
