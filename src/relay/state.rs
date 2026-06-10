pub use crate::scheduler::selector::{GroupInfo, GroupItemInfo};

use crate::api::handlers::admin::channels::{
    CustomHeader, EndpointConfig, EndpointType, UpstreamApiKey, parse_api_keys,
};
use crate::metrics::model::ModelRegistry;
use crate::metrics::recorder::StatsRecorder;
use crate::protocol::outbound::Outbound;
use crate::protocol::sse::sanitize_upstream_error;
use crate::relay::cache::ProxyCache;
use crate::relay::channel::ChannelInfo;
use crate::relay::queue::RequestQueue;
use crate::relay::ratelimit::RateLimiter;
use crate::relay::run::RelayCandidate;
use crate::scheduler::scoring::{
    CandidateScoreInput, SchedulerScoreWeights, ScoredCandidate, select_top_k_candidates,
};
use crate::scheduler::state::LoadBalancerState;
use axum::http::{HeaderMap, StatusCode};
use sqlx::SqlitePool;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// 代理状态
#[derive(Clone)]
pub struct ProxyState {
    pub pool: SqlitePool,
    pub http_client: reqwest::Client,
    pub lb_state: LoadBalancerState,
    pub stats_recorder: StatsRecorder,
    pub model_registry: ModelRegistry,
    pub cache: ProxyCache,
    pub rate_limiter: RateLimiter,
    pub queue: Option<RequestQueue>,
    key_counter: Arc<AtomicU64>,
}

/// 代理成功结果（非流式）
pub struct ProxySuccess {
    pub status: StatusCode,
    pub body: Vec<u8>,
}

impl ProxyState {
    pub async fn new(pool: SqlitePool, model_registry: ModelRegistry) -> Self {
        let proxy_enabled: bool = sqlx::query_scalar::<_, String>(
            "SELECT value FROM settings WHERE key = 'proxy.enabled'",
        )
        .fetch_optional(&pool)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(false);

        let proxy_url = if proxy_enabled {
            sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = 'proxy.url'")
                .fetch_optional(&pool)
                .await
                .ok()
                .flatten()
                .filter(|v| !v.is_empty())
        } else {
            None
        };

        let mut client_builder =
            reqwest::Client::builder().timeout(std::time::Duration::from_secs(300));

        if let Some(url) = proxy_url {
            match reqwest::Proxy::all(&url) {
                Ok(proxy) => {
                    tracing::info!("上游代理已启用: {}", url);
                    client_builder = client_builder.proxy(proxy);
                }
                Err(e) => {
                    tracing::warn!("代理配置无效，忽略代理: {}", e);
                    client_builder = client_builder.no_proxy();
                }
            }
        } else {
            client_builder = client_builder.no_proxy();
        }

        let http_client = client_builder
            .build()
            .expect("Failed to create HTTP client");

        Self {
            stats_recorder: StatsRecorder::new(pool.clone()),
            model_registry,
            cache: ProxyCache::new(),
            rate_limiter: RateLimiter::new(),
            queue: None,
            pool,
            http_client,
            lb_state: LoadBalancerState::new(),
            key_counter: Arc::new(AtomicU64::new(0)),
        }
    }

    /// 设置请求队列
    pub fn with_queue(mut self, max_queue_size: usize, timeout_secs: u64) -> Self {
        self.queue = Some(RequestQueue::new(max_queue_size, timeout_secs));
        self
    }

    /// 根据名称查找分组（带缓存）
    async fn find_group_by_name(&self, name: &str) -> Result<Option<GroupInfo>, ProxyError> {
        // 1. 检查缓存
        if let Some(group) = self.cache.get_group(name).await {
            return Ok(Some(group));
        }

        // 2. 缓存未命中，查询数据库
        let result = sqlx::query_as::<_, (String, String)>(
            "SELECT id, name FROM groups WHERE name = ? AND enabled = 1",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        match result {
            Some((id, name)) => {
                let items = self.get_group_items(&id).await?;
                let group = GroupInfo {
                    id,
                    name: name.clone(),
                    items,
                };
                // 3. 写入缓存
                self.cache.set_group(group.clone()).await;
                Ok(Some(group))
            }
            None => Ok(None),
        }
    }

    /// 根据正则查找分组
    async fn find_group_by_regex(&self, model: &str) -> Result<Option<GroupInfo>, ProxyError> {
        let groups = sqlx::query_as::<_, (String, String, Option<String>)>(
            "SELECT id, name, match_regex FROM groups WHERE enabled = 1 AND match_regex IS NOT NULL"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        for (id, name, match_regex) in groups {
            if let Some(pattern) = match_regex
                && let Some(re) = self.cache.get_compiled_regex(&pattern).await
                && re.is_match(model)
            {
                let items = self.get_group_items(&id).await?;
                return Ok(Some(GroupInfo { id, name, items }));
            }
        }

        Ok(None)
    }

    /// 获取分组项
    async fn get_group_items(&self, group_id: &str) -> Result<Vec<GroupItemInfo>, ProxyError> {
        let items = sqlx::query_as::<_, (String, String, i32, i32)>(
            "SELECT channel_id, model_name, priority, weight FROM group_items WHERE group_id = ? ORDER BY priority ASC, weight DESC"
        )
        .bind(group_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        Ok(items
            .into_iter()
            .map(|(channel_id, model_name, priority, weight)| GroupItemInfo {
                channel_id,
                model_name,
                priority,
                weight,
            })
            .collect())
    }
}

/// 获取渠道信息（带缓存）
impl ProxyState {
    pub(crate) async fn get_channel(&self, channel_id: &str) -> Result<ChannelInfo, ProxyError> {
        // 1. 检查缓存
        if let Some(channel) = self.cache.get_channel(channel_id).await {
            return Ok(channel);
        }

        // 2. 缓存未命中，查询数据库
        let result = sqlx::query_as::<_, (String, String, String, String, String, String, i32, i32, String)>(
            "SELECT id, name, api_keys, endpoints, models, custom_headers, COALESCE(timeout_secs, 300), COALESCE(max_concurrency, 0), extras FROM channels WHERE id = ? AND enabled = 1"
        )
        .bind(channel_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        let (
            id,
            name,
            api_keys_str,
            endpoints_str,
            models_str,
            custom_headers_str,
            timeout_secs,
            max_concurrency,
            extras_str,
        ) = result.ok_or_else(|| ProxyError::ChannelNotFound("渠道不存在或已禁用".to_string()))?;

        let api_keys: Vec<UpstreamApiKey> = parse_api_keys(&api_keys_str);
        let endpoints: Vec<EndpointConfig> =
            serde_json::from_str(&endpoints_str).unwrap_or_default();
        let models = parse_models(&models_str);
        let custom_headers: Vec<CustomHeader> =
            serde_json::from_str(&custom_headers_str).unwrap_or_default();
        let extras: Option<serde_json::Map<String, serde_json::Value>> =
            serde_json::from_str(&extras_str).ok();

        let channel = ChannelInfo {
            id,
            name,
            api_keys,
            endpoints,
            models,
            custom_headers,
            timeout_secs: timeout_secs as u64,
            max_concurrency: max_concurrency as u32,
            extras,
        };

        // 3. 写入缓存
        self.cache.set_channel(channel.clone()).await;

        Ok(channel)
    }

    /// 生成一次请求内的同渠道 Key 尝试序列（跳过禁用 Key）。
    pub fn api_key_attempts(&self, channel: &ChannelInfo) -> Vec<String> {
        let enabled_keys = channel.enabled_api_keys();
        if enabled_keys.is_empty() {
            return vec![String::new()];
        }

        let start = self.key_counter.fetch_add(1, Ordering::Relaxed) as usize % enabled_keys.len();

        (0..enabled_keys.len())
            .map(|offset| {
                enabled_keys[(start + offset) % enabled_keys.len()]
                    .key
                    .clone()
            })
            .collect()
    }
}

/// 获取出站转换器（静态引用，避免堆分配）
pub fn get_outbound(endpoint_type: &EndpointType) -> &'static dyn Outbound {
    crate::protocol::outbound::outbound_for(endpoint_type)
        .unwrap_or(&crate::protocol::outbound::openai_chat::OpenAiChatOutbound)
}

/// 错误格式类型
pub enum ErrorFormat {
    /// OpenAI 格式: {"error": {"message": ..., "type": ...}}
    OpenAi,
    /// Anthropic 格式: {"type": "error", "error": {"type": ..., "message": ...}}
    Anthropic,
}

/// 代理错误
#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("数据库错误: {0}")]
    DatabaseError(String),

    #[error("渠道不存在: {0}")]
    ChannelNotFound(String),

    #[error("没有可用渠道: {0}")]
    NoAvailableChannel(String),

    #[error("模型不支持: {0}")]
    ModelNotSupported(String),

    #[error("模型不存在: {0}")]
    ModelNotFound(String),

    #[error("请求失败: {0}")]
    RequestError(String),

    #[error("转换失败: {0}")]
    TransformError(String),

    #[error("上游错误: {status}")]
    UpstreamError { status: StatusCode, body: String },
}

/// 错误分类（用于重试与降级策略）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// 上游 Key 相关问题（401、402、429、余额不足、无效 key 等）— 换 Key
    KeyRetryable,
    /// 上游临时错误（5xx、超时）— 可换渠道
    UpstreamRetryable,
    /// 客户端错误（4xx、非 Key 鉴权）— 不重试
    Client,
    /// 系统/内部错误 — 不重试
    Internal,
}

impl ProxyError {
    /// 错误分类
    pub fn classify(&self) -> ErrorClass {
        match self {
            ProxyError::DatabaseError(_) | ProxyError::ChannelNotFound(_) => ErrorClass::Internal,
            ProxyError::NoAvailableChannel(_) | ProxyError::RequestError(_) => {
                ErrorClass::UpstreamRetryable
            }
            ProxyError::ModelNotSupported(_) | ProxyError::ModelNotFound(_) => ErrorClass::Client,
            ProxyError::TransformError(_) => ErrorClass::Internal,
            ProxyError::UpstreamError { status, body } => classify_upstream(*status, body),
        }
    }

    /// 上游错误是否应当换 Key 重试
    pub fn is_key_retryable(&self) -> bool {
        matches!(self.classify(), ErrorClass::KeyRetryable)
    }

    /// 从 RelayRunOutcome 构建错误
    pub fn from_relay_outcome(outcome: &crate::relay::run::RelayRunOutcome) -> Self {
        match outcome.status_code {
            404 => ProxyError::ModelNotFound(
                outcome
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "模型不存在".to_string()),
            ),
            _ => {
                if let Some(ref msg) = outcome.error_message {
                    if outcome.status_code >= 500 {
                        ProxyError::UpstreamError {
                            status: StatusCode::from_u16(outcome.status_code)
                                .unwrap_or(StatusCode::BAD_GATEWAY),
                            body: msg.clone(),
                        }
                    } else {
                        ProxyError::NoAvailableChannel(msg.clone())
                    }
                } else {
                    ProxyError::NoAvailableChannel("所有渠道都不可用".to_string())
                }
            }
        }
    }
    /// 从 RelayStreamRunOutcome 构建错误
    pub fn from_relay_stream_outcome(outcome: &crate::relay::run::RelayStreamRunOutcome) -> Self {
        match outcome.status_code {
            404 => ProxyError::ModelNotFound(
                outcome
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "模型不存在".to_string()),
            ),
            _ => {
                if let Some(ref msg) = outcome.error_message {
                    if outcome.status_code >= 500 {
                        ProxyError::UpstreamError {
                            status: StatusCode::from_u16(outcome.status_code)
                                .unwrap_or(StatusCode::BAD_GATEWAY),
                            body: msg.clone(),
                        }
                    } else {
                        ProxyError::NoAvailableChannel(msg.clone())
                    }
                } else {
                    ProxyError::NoAvailableChannel("所有渠道都不可用".to_string())
                }
            }
        }
    }
}

fn classify_upstream(status: StatusCode, body: &str) -> ErrorClass {
    if matches!(
        status,
        StatusCode::UNAUTHORIZED | StatusCode::PAYMENT_REQUIRED | StatusCode::TOO_MANY_REQUESTS
    ) {
        return ErrorClass::KeyRetryable;
    }

    let lower = sanitize_upstream_error(body).to_ascii_lowercase();
    const KEY_NEEDLES: &[&str] = &[
        "余额不足",
        "无可用资源包",
        "insufficient_quota",
        "quota exceeded",
        "resource exhausted",
        "credit balance",
        "billing",
        "rate limit",
        "invalid api key",
        "incorrect api key",
        "unauthorized",
        "authentication",
    ];
    if KEY_NEEDLES.iter().any(|n| lower.contains(n)) {
        return ErrorClass::KeyRetryable;
    }

    if status.is_server_error() {
        ErrorClass::UpstreamRetryable
    } else {
        ErrorClass::Client
    }
}

/// M3-S1: 将候选渠道转换为 CandidateScoreInput 并用 scheduler 多因子打分排序
async fn score_candidates(
    lb_state: &LoadBalancerState,
    candidates: &[&GroupItemInfo],
) -> Vec<ScoredCandidate> {
    let states = lb_state.channel_states.read().await;

    let inputs: Vec<CandidateScoreInput> = candidates
        .iter()
        .map(|item| {
            let status = states.get(&item.channel_id);
            let runtime = lb_state.runtime_stats(&item.channel_id);
            let runtime_latency = if runtime.avg_ttft_ms() > 0.0 {
                Some(runtime.avg_ttft_ms())
            } else if runtime.avg_latency_ms() > 0.0 {
                Some(runtime.avg_latency_ms())
            } else {
                None
            };
            CandidateScoreInput {
                candidate_id: item.channel_id.clone(),
                priority: item.priority,
                load_rate: status.map(|s| s.load_rate()).unwrap_or(0),
                waiting_count: 0,
                max_waiting_count: 0,
                error_rate: runtime.error_rate(),
                latency_ms: runtime_latency,
                min_latency_ms: None,
                max_latency_ms: None,
                health: runtime.health()
                    * status
                        .map(|s| if s.is_available() { 1.0 } else { 0.0 })
                        .unwrap_or(1.0),
            }
        })
        .collect();

    let top_k = inputs.len().min(3);
    select_top_k_candidates(&inputs, top_k, &SchedulerScoreWeights::default())
}

/// 构建有序候选列表（sticky + scored group items）
///
/// 流程：
/// 1. 检查 sticky session → 加入候选（sticky=true, score=最高）
/// 2. 查找分组 → 获取 group_items → score_candidates 打分排序
/// 3. 构建 Vec<RelayCandidate>（sticky 在前，按 score 降序）
pub(crate) async fn build_relay_candidates(
    state: &ProxyState,
    model: &str,
    client_endpoint: &EndpointType,
    session_hash: Option<&str>,
) -> Result<Vec<RelayCandidate>, ProxyError> {
    let mut candidates = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // 1. Sticky session
    if let Some(hash) = session_hash
        && let Some(channel_id) = state.lb_state.get_sticky_session(hash).await
        && let Ok(channel) = state.get_channel(&channel_id).await
        && channel.find_endpoint(client_endpoint).is_some()
    {
        seen.insert(channel_id.clone());
        candidates.push(RelayCandidate {
            channel_id: channel_id.clone(),
            channel_name: channel.name.clone(),
            max_concurrency: channel.max_concurrency,
            score: 100.0,
            sticky: true,
            target_model: model.to_string(),
            group_id: None,
        });
    }

    // 2. 分组候选（精确端点匹配）
    let group = match state.find_group_by_name(model).await? {
        Some(g) => Some(g),
        None => state.find_group_by_regex(model).await?,
    };

    if let Some(ref group) = group {
        let items: Vec<&GroupItemInfo> = group
            .items
            .iter()
            .filter(|item| !seen.contains(&item.channel_id))
            .collect();

        if !items.is_empty() {
            let scored = score_candidates(&state.lb_state, &items).await;
            for sc in &scored {
                if seen.insert(sc.input.candidate_id.clone()) {
                    let max_concurrency = state
                        .get_channel(&sc.input.candidate_id)
                        .await
                        .map(|ch| ch.max_concurrency)
                        .unwrap_or(0);
                    let target_model = items
                        .iter()
                        .find(|it| it.channel_id == sc.input.candidate_id)
                        .map(|it| it.model_name.clone())
                        .unwrap_or_else(|| model.to_string());

                    candidates.push(RelayCandidate {
                        channel_id: sc.input.candidate_id.clone(),
                        channel_name: String::new(),
                        max_concurrency,
                        score: sc.score,
                        sticky: false,
                        target_model,
                        group_id: Some(group.id.clone()),
                    });
                }
            }
        }
    }

    // 3. 候选为空时区分 "模型不存在" vs "渠道不可用"
    if candidates.is_empty() {
        return if group.is_some() {
            Err(ProxyError::NoAvailableChannel(format!(
                "模型 {} 的所有渠道不可用",
                model
            )))
        } else {
            Err(ProxyError::ModelNotFound(format!("模型不存在: {}", model)))
        };
    }

    Ok(candidates)
}

/// 解析 models 字段
fn parse_models(models_str: &str) -> Vec<String> {
    serde_json::from_str(models_str).unwrap_or_default()
}

/// 验证字符串可作为 HTTP header value
/// 用于在保存上游 API Key 时一次性拦截含 CRLF / 控制字符的输入，
/// 避免转发时 `HeaderValue::from_str(...).unwrap()` panic。
pub(crate) fn validate_header_value(s: &str) -> Result<(), String> {
    reqwest::header::HeaderValue::from_str(s)
        .map(|_| ())
        .map_err(|e| format!("含非法 header 字符 ({e})"))
}

/// 检查 API Key 的预算额度（月/日）
async fn check_budget(pool: &SqlitePool, key_id: &str) -> Result<(), String> {
    let limit: Option<(f64, f64)> = sqlx::query_as(
        "SELECT monthly_limit_usd, daily_limit_usd FROM budget_limits WHERE api_key_id = ? AND enabled = 1",
    )
    .bind(key_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("查询预算失败: {}", e))?;

    let Some((monthly_limit, daily_limit)) = limit else {
        return Ok(()); // 无预算限制
    };

    // 查询累计消费
    let (monthly_cost, daily_cost): (f64, f64) = sqlx::query_as(
        r#"SELECT
            COALESCE(SUM(COALESCE(cost, 0)), 0),
            COALESCE(SUM(CASE WHEN date(created_at) = date('now') THEN COALESCE(cost, 0) ELSE 0 END), 0)
        FROM usage_logs WHERE api_key_id = ?"#,
    )
    .bind(key_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("查询消费失败: {}", e))?;

    if daily_limit > 0.0 && daily_cost >= daily_limit {
        return Err(format!(
            "日预算已耗尽: ${:.2}/${:.2}",
            daily_cost, daily_limit
        ));
    }
    if monthly_limit > 0.0 && monthly_cost >= monthly_limit {
        return Err(format!(
            "月预算已耗尽: ${:.2}/${:.2}",
            monthly_cost, monthly_limit
        ));
    }

    Ok(())
}

/// 验证 API Key 是否有权访问目标模型（三段式：403→404→503）
async fn validate_model_access(
    pool: &SqlitePool,
    key_id: &str,
    model: &str,
    allowed_groups: &str,
) -> Result<(), ProxyError> {
    // === 第一关：检查 allowed_groups / supported_models ===
    // allowed_groups 非空时，检查请求的 model 是否在允许的分组列表中
    if !allowed_groups.is_empty() {
        let allowed: Vec<&str> = allowed_groups
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if !allowed.is_empty() && !allowed.contains(&model) {
            return Err(ProxyError::ModelNotSupported(format!(
                "API Key 无权访问模型: {}",
                model
            )));
        }
    } else {
        // allowed_groups 为空时回退到 supported_models（兼容旧逻辑）
        let supported = sqlx::query_scalar::<_, String>(
            "SELECT supported_models FROM api_keys WHERE id = ? AND enabled = 1",
        )
        .bind(key_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        if let Some(models_str) = supported
            && !models_str.is_empty()
        {
            let allowed =
                crate::api::handlers::admin::api_keys::parse_supported_models(&models_str);
            if !allowed.iter().any(|m| m == model) {
                return Err(ProxyError::ModelNotSupported(format!(
                    "API Key 无权访问模型: {}",
                    model
                )));
            }
        }
    }

    // === 第二关：检查分组是否存在 ===
    // 仅在 allowed_groups 非空时严格检查（Octopus 策略：明确指定了分组才校验）
    // allowed_groups 为空时跳过，由后续 Relay candidate 构建检查渠道可用性
    if !allowed_groups.is_empty() {
        let group_exists: bool = sqlx::query_scalar::<_, i32>(
            "SELECT COUNT(*) FROM groups WHERE name = ? AND enabled = 1",
        )
        .bind(model)
        .fetch_one(pool)
        .await
        .map(|c| c > 0)
        .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        if !group_exists {
            return Err(ProxyError::ModelNotFound(format!("模型不存在: {}", model)));
        }
    }

    // === 第三关：分组内是否有可用渠道（延迟到 Relay candidate 构建时检查）===
    Ok(())
}

/// 格式化 402 预算超限错误
fn format_budget_error(msg: &str, error_format: &ErrorFormat) -> axum::response::Response {
    use axum::response::IntoResponse;

    let body = match error_format {
        ErrorFormat::OpenAi => serde_json::json!({
            "error": {
                "message": msg,
                "type": "insufficient_quota",
                "code": "budget_exceeded"
            }
        }),
        ErrorFormat::Anthropic => serde_json::json!({
            "type": "error",
            "error": { "type": "insufficient_quota", "message": msg }
        }),
    };

    (StatusCode::PAYMENT_REQUIRED, axum::Json(body)).into_response()
}

/// 格式化代理错误为 HTTP 响应
pub fn format_proxy_error(e: ProxyError, format: &ErrorFormat) -> axum::response::Response {
    use axum::response::IntoResponse;

    let (status, message) = render_status_and_message(&e);
    let body = match format {
        ErrorFormat::OpenAi => serde_json::json!({
            "error": { "message": message, "type": "server_error" }
        }),
        ErrorFormat::Anthropic => serde_json::json!({
            "type": "error",
            "error": { "type": "api_error", "message": message }
        }),
    };
    (status, axum::Json(body)).into_response()
}

/// 格式化 429 速率限制错误
fn format_rate_limit_error(
    retry_after: f64,
    limit_type: &str,
    limit: u64,
    error_format: &ErrorFormat,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let message = format!(
        "速率限制 exceeded: {} limit={}，请 {:.0} 秒后重试",
        limit_type, limit, retry_after
    );
    let body = match error_format {
        ErrorFormat::OpenAi => serde_json::json!({
            "error": {
                "message": message,
                "type": "rate_limit_error",
                "code": "rate_limit_exceeded"
            }
        }),
        ErrorFormat::Anthropic => serde_json::json!({
            "type": "error",
            "error": { "type": "rate_limit_error", "message": message }
        }),
    };

    (
        StatusCode::TOO_MANY_REQUESTS,
        [
            ("Retry-After", format!("{:.0}", retry_after)),
            ("X-RateLimit-Limit", limit.to_string()),
        ],
        axum::Json(body),
    )
        .into_response()
}

/// 提取 (HTTP 状态码, 客户端可见消息) — 与客户端格式无关
fn render_status_and_message(e: &ProxyError) -> (StatusCode, String) {
    match e {
        ProxyError::NoAvailableChannel(msg) => (StatusCode::SERVICE_UNAVAILABLE, msg.clone()),
        ProxyError::ModelNotSupported(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
        ProxyError::ModelNotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
        ProxyError::UpstreamError { status, body } => (*status, sanitize_upstream_error(body)),
        _ => (StatusCode::BAD_GATEWAY, e.to_string()),
    }
}

/// 兼容封装：仅根据 status + body 判断上游错误是否需要换 Key
///
/// 内部委托给 `ProxyError::is_key_retryable`，避免散落的多处独立判断。
#[allow(dead_code)]
fn is_key_retryable_upstream_error(status: StatusCode, body: &str) -> bool {
    ProxyError::UpstreamError {
        status,
        body: body.to_string(),
    }
    .is_key_retryable()
}

/// 非流式代理请求（支持重试和排队）
pub async fn proxy_request(
    state: &ProxyState,
    request_id: &str,
    api_key_id: Option<&str>,
    headers: &HeaderMap,
    body: &serde_json::Value,
    client_endpoint: &EndpointType,
) -> Result<ProxySuccess, ProxyError> {
    use crate::metrics::recorder::redaction::sanitize_json_content;
    use crate::metrics::recorder::save_request_record;

    let _permit = if let Some(queue) = &state.queue {
        Some(
            queue
                .acquire()
                .await
                .map_err(|e| ProxyError::RequestError(format!("排队失败: {}", e)))?,
        )
    } else {
        None
    };

    let model = body["model"].as_str().unwrap_or("unknown").to_string();
    let request_content = serde_json::to_string(&body)
        .ok()
        .map(|c| sanitize_json_content(&c));
    let user_agent = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let session_hash = headers
        .get("x-session-hash")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let sticky_channel_id = if let Some(hash) = session_hash.as_deref() {
        state.lb_state.get_sticky_session(hash).await
    } else {
        None
    };

    // 1. 构建候选列表（sticky + scored group items）
    let candidates =
        match build_relay_candidates(state, &model, client_endpoint, session_hash.as_deref()).await
        {
            Ok(c) => c,
            Err(e) => {
                save_request_record(
                    state,
                    Some(request_id.to_string()),
                    api_key_id,
                    None,
                    &model,
                    request_content,
                    None,
                    &[],
                    None,
                    false,
                    user_agent,
                    Some(&e),
                )
                .await;
                return Err(e);
            }
        };

    // 2. 创建 executor + RelayRun
    let executor = crate::relay::executor::ProxyRelayExecutor::new(
        state.clone(),
        headers.clone(),
        body.clone(),
        client_endpoint.clone(),
        api_key_id.map(|s| s.to_string()),
    );

    let capacity = state.lb_state.capacity_manager();
    let run = crate::relay::run::RelayRun::new(capacity, executor.clone());
    let relay_request = crate::relay::run::RelayRequest::new(&model);

    // 3. 执行
    let candidate_snapshot = candidates.clone();
    let outcome = run.execute(relay_request, candidates).await;
    let attempt_stats = executor.take_attempt_stats();

    // 4. 记录日志 + 返回结果
    if let Some(response_body) = outcome.response {
        if let Some(channel_id) = outcome.selected_channel_id.as_deref() {
            let selected_was_sticky = candidate_snapshot
                .iter()
                .any(|c| c.channel_id == channel_id && c.sticky);
            state.lb_state.record_scheduler_selection(
                channel_id,
                selected_was_sticky,
                sticky_channel_id.as_deref(),
            );
            if let Some(hash) = session_hash.as_deref() {
                state.lb_state.set_sticky_session(hash, channel_id).await;
            }
        }
        save_request_record(
            state,
            Some(request_id.to_string()),
            api_key_id,
            None,
            &model,
            request_content,
            Some(response_body.clone()),
            &attempt_stats,
            None,
            false,
            user_agent,
            None,
        )
        .await;

        Ok(ProxySuccess {
            status: StatusCode::OK,
            body: response_body.into_bytes(),
        })
    } else {
        let error = ProxyError::from_relay_outcome(&outcome);
        save_request_record(
            state,
            Some(request_id.to_string()),
            api_key_id,
            None,
            &model,
            request_content,
            None,
            &attempt_stats,
            None,
            false,
            user_agent,
            Some(&error),
        )
        .await;
        Err(error)
    }
}

/// 流式代理请求（支持重试和排队）
pub async fn proxy_stream(
    state: &ProxyState,
    request_id: &str,
    api_key_id: Option<&str>,
    headers: &HeaderMap,
    body: &serde_json::Value,
    client_endpoint: &EndpointType,
) -> Result<
    (
        StatusCode,
        std::pin::Pin<
            Box<
                dyn futures::Stream<Item = Result<axum::body::Bytes, std::convert::Infallible>>
                    + Send
                    + 'static,
            >,
        >,
        String,
    ),
    ProxyError,
> {
    use crate::metrics::recorder::save_request_record;

    let queue_permit = if let Some(queue) = &state.queue {
        Some(
            queue
                .acquire()
                .await
                .map_err(|e| ProxyError::RequestError(format!("排队失败: {}", e)))?,
        )
    } else {
        None
    };

    let model = body["model"].as_str().unwrap_or("unknown").to_string();
    let request_content = serde_json::to_string(&body).ok();
    let user_agent = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let session_hash = headers
        .get("x-session-hash")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let sticky_channel_id = if let Some(hash) = session_hash.as_deref() {
        state.lb_state.get_sticky_session(hash).await
    } else {
        None
    };

    let candidates =
        match build_relay_candidates(state, &model, client_endpoint, session_hash.as_deref()).await
        {
            Ok(c) => c,
            Err(e) => {
                save_request_record(
                    state,
                    Some(request_id.to_string()),
                    api_key_id,
                    None,
                    &model,
                    request_content,
                    None,
                    &[],
                    None,
                    true,
                    user_agent,
                    Some(&e),
                )
                .await;
                return Err(e);
            }
        };

    let executor = crate::relay::stream_executor::ProxyStreamRelayExecutor::new(
        state.clone(),
        request_id.to_string(),
        headers.clone(),
        body.clone(),
        client_endpoint.clone(),
        api_key_id.map(|s| s.to_string()),
        queue_permit,
    );
    let run =
        crate::relay::run::RelayStreamRun::new(state.lb_state.capacity_manager(), executor.clone());
    let candidate_snapshot = candidates.clone();
    let outcome = run
        .execute(crate::relay::run::RelayRequest::new(&model), candidates)
        .await;

    if let Some(success) = outcome.response {
        if let Some(channel_id) = outcome.selected_channel_id.as_deref() {
            let selected_was_sticky = candidate_snapshot
                .iter()
                .any(|c| c.channel_id == channel_id && c.sticky);
            state.lb_state.record_scheduler_selection(
                channel_id,
                selected_was_sticky,
                sticky_channel_id.as_deref(),
            );
            if let Some(hash) = session_hash.as_deref() {
                state.lb_state.set_sticky_session(hash, channel_id).await;
            }
        }
        return Ok((success.status, success.stream, success.content_type));
    }

    let attempt_stats = executor.take_attempt_stats();
    let error = ProxyError::from_relay_stream_outcome(&outcome);
    save_request_record(
        state,
        Some(request_id.to_string()),
        api_key_id,
        None,
        &model,
        request_content,
        None,
        &attempt_stats,
        None,
        true,
        user_agent,
        Some(&error),
    )
    .await;
    Err(error)
}

/// 统一代理请求入口（供各 handler 调用）
pub async fn handle_proxy_request(
    state: &ProxyState,
    auth: crate::api::middleware::ApiKeyAuth,
    headers: HeaderMap,
    body: serde_json::Value,
    client_endpoint: &EndpointType,
    error_format: &ErrorFormat,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let request_id = crate::api::response::generate_id();
    let model = body["model"].as_str().unwrap_or("unknown");
    let is_stream = body["stream"].as_bool().unwrap_or(false);
    let api_key_id = Some(auth.key_id.as_str());

    // 速率限制检查（RPM + TPM）
    if let Err(retry_after) = state
        .rate_limiter
        .check_rpm(&auth.key_id, auth.rate_limit_rpm)
        .await
    {
        return format_rate_limit_error(retry_after, "RPM", auth.rate_limit_rpm, error_format);
    }
    if let Err(retry_after) = state
        .rate_limiter
        .check_tpm(&auth.key_id, auth.rate_limit_tpm)
        .await
    {
        return format_rate_limit_error(retry_after, "TPM", auth.rate_limit_tpm, error_format);
    }

    // 预算检查（月/日额度）
    if let Err(msg) = check_budget(&state.pool, &auth.key_id).await {
        return format_budget_error(&msg, error_format);
    }

    // 验证 API Key 是否有权访问目标模型（三段式：403→404→503）
    if let Err(e) =
        validate_model_access(&state.pool, &auth.key_id, model, &auth.allowed_groups).await
    {
        return format_proxy_error(e, error_format);
    }

    if is_stream {
        match proxy_stream(
            state,
            &request_id,
            api_key_id,
            &headers,
            &body,
            client_endpoint,
        )
        .await
        {
            Ok((status, stream, content_type)) => axum::response::Response::builder()
                .status(status)
                .header("Content-Type", content_type)
                .header("Cache-Control", "no-cache")
                .header("Connection", "keep-alive")
                .header("X-Request-ID", &request_id)
                .body(axum::body::Body::from_stream(stream))
                .expect("static headers + StatusCode from upstream are valid Response inputs")
                .into_response(),
            Err(e) => format_proxy_error(e, error_format),
        }
    } else {
        match proxy_request(
            state,
            &request_id,
            api_key_id,
            &headers,
            &body,
            client_endpoint,
        )
        .await
        {
            Ok(result) => axum::response::Response::builder()
                .status(result.status)
                .header("Content-Type", "application/json")
                .header("X-Request-ID", &request_id)
                .body(axum::body::Body::from(result.body))
                .expect("static Content-Type + StatusCode are valid Response inputs")
                .into_response(),
            Err(e) => format_proxy_error(e, error_format),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_retryable_error_matches_status_and_quota_body() {
        assert!(is_key_retryable_upstream_error(
            StatusCode::UNAUTHORIZED,
            r#"{"error":{"message":"bad key"}}"#
        ));
        assert!(is_key_retryable_upstream_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"error":{"message":"insufficient_quota"}}"#
        ));
        assert!(is_key_retryable_upstream_error(
            StatusCode::BAD_REQUEST,
            r#"{"error":{"message":"余额不足或无可用资源包"}}"#
        ));
    }

    #[test]
    fn key_retryable_error_ignores_non_key_errors() {
        assert!(!is_key_retryable_upstream_error(
            StatusCode::BAD_REQUEST,
            r#"{"error":{"message":"model does not exist"}}"#
        ));
        assert!(!is_key_retryable_upstream_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"error":{"message":"upstream overloaded"}}"#
        ));
    }

    #[test]
    fn validate_header_value_accepts_normal_and_rejects_crlf() {
        assert!(validate_header_value("sk-abc.123_OK-9").is_ok());
        assert!(validate_header_value("sk-abc/def+ghi=").is_ok());
        assert!(validate_header_value("sk-abc").is_ok());
        assert!(validate_header_value("sk-abc\r\nfoo").is_err());
        assert!(validate_header_value("sk-abc\0").is_err());
        assert!(validate_header_value("sk-abc\x7f").is_err());
    }

    #[test]
    fn classify_upstream_distinguishes_key_retryable() {
        // Key 相关：401 / 402 / 429
        assert_eq!(
            classify_upstream(StatusCode::UNAUTHORIZED, ""),
            ErrorClass::KeyRetryable
        );
        assert_eq!(
            classify_upstream(StatusCode::TOO_MANY_REQUESTS, ""),
            ErrorClass::KeyRetryable
        );
        // 5xx 非 Key 字样：可换渠道重试
        assert_eq!(
            classify_upstream(StatusCode::INTERNAL_SERVER_ERROR, "upstream overloaded"),
            ErrorClass::UpstreamRetryable
        );
        // 4xx 非 Key 字样：客户端错误
        assert_eq!(
            classify_upstream(StatusCode::BAD_REQUEST, "model does not exist"),
            ErrorClass::Client
        );
        // 5xx 含余额字样：Key 重试优先
        assert_eq!(
            classify_upstream(StatusCode::INTERNAL_SERVER_ERROR, "insufficient_quota"),
            ErrorClass::KeyRetryable
        );
    }

    #[test]
    fn render_status_and_message_maps_error_kinds() {
        use axum::http::StatusCode;
        // NoAvailableChannel → 503 + 原 message
        let (s, m) = render_status_and_message(&ProxyError::NoAvailableChannel("no ch".into()));
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(m, "no ch");

        // UpstreamError → 上游 status + sanitized body
        let (s, m) = render_status_and_message(&ProxyError::UpstreamError {
            status: StatusCode::TOO_MANY_REQUESTS,
            body: r#"{"error":{"message":"rate limit"}}"#.into(),
        });
        assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
        assert!(m.contains("rate limit"));

        // 其他 → 502 + thiserror 格式化
        let (s, m) = render_status_and_message(&ProxyError::DatabaseError("db gone".into()));
        assert_eq!(s, StatusCode::BAD_GATEWAY);
        assert!(m.contains("db gone"));
    }

    // ============================================================
    // M3-S1: scheduler scoring integration characterization
    // ============================================================

    /// 验证多因子打分：健康 + 低延迟渠道排在前面
    #[tokio::test]
    async fn scheduler_selection_integration_prefers_healthy_low_latency() {
        let lb = LoadBalancerState::new();
        lb.ensure_channel_status("ch-fast", 10).await;
        lb.ensure_channel_status("ch-slow", 10).await;

        // ch-fast: 低延迟、无错误
        lb.record_success("ch-fast", 50.0).await;

        // ch-slow: 高延迟、有失败记录
        lb.record_success("ch-slow", 500.0).await;
        lb.record_failure("ch-slow", false).await;
        lb.record_failure("ch-slow", false).await;

        let items = vec![
            GroupItemInfo {
                channel_id: "ch-fast".into(),
                model_name: "gpt-4o".into(),
                priority: 1,
                weight: 10,
            },
            GroupItemInfo {
                channel_id: "ch-slow".into(),
                model_name: "gpt-4o".into(),
                priority: 1,
                weight: 10,
            },
        ];
        let refs: Vec<&GroupItemInfo> = items.iter().collect();

        let scored = score_candidates(&lb, &refs).await;

        assert_eq!(scored.len(), 2);
        assert_eq!(scored[0].input.candidate_id, "ch-fast");
        assert!(
            scored[0].score > scored[1].score,
            "healthy candidate should score higher"
        );
    }

    /// 验证容量惩罚：高负载渠道得分更低
    #[tokio::test]
    async fn scheduler_selection_integration_full_capacity_load_penalty() {
        let lb = LoadBalancerState::new();
        lb.ensure_channel_status("ch-idle", 10).await;
        lb.ensure_channel_status("ch-busy", 10).await;

        // ch-busy: 9/10 负载
        for _ in 0..9 {
            lb.increment_active("ch-busy").await;
        }

        let items = vec![
            GroupItemInfo {
                channel_id: "ch-idle".into(),
                model_name: "gpt-4o".into(),
                priority: 1,
                weight: 10,
            },
            GroupItemInfo {
                channel_id: "ch-busy".into(),
                model_name: "gpt-4o".into(),
                priority: 1,
                weight: 10,
            },
        ];
        let refs: Vec<&GroupItemInfo> = items.iter().collect();

        let scored = score_candidates(&lb, &refs).await;

        assert_eq!(scored[0].input.candidate_id, "ch-idle");
        assert!(
            scored[0].score > scored[1].score,
            "idle candidate should score higher than busy"
        );
        assert_eq!(scored[1].input.load_rate, 90);
    }

    /// 验证优先级：低优先级数值（高优先）的渠道排在前面
    #[tokio::test]
    async fn scheduler_selection_integration_priority_takes_precedence() {
        let lb = LoadBalancerState::new();

        let items = vec![
            GroupItemInfo {
                channel_id: "ch-low-prio".into(),
                model_name: "gpt-4o".into(),
                priority: 5,
                weight: 10,
            },
            GroupItemInfo {
                channel_id: "ch-high-prio".into(),
                model_name: "gpt-4o".into(),
                priority: 1,
                weight: 5,
            },
        ];
        let refs: Vec<&GroupItemInfo> = items.iter().collect();

        let scored = score_candidates(&lb, &refs).await;

        assert_eq!(scored[0].input.candidate_id, "ch-high-prio");
        assert!(
            scored[0].score > scored[1].score,
            "high priority (low value) should score higher"
        );
    }
}
