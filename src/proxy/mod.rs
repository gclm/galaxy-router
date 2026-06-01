pub mod cache;
pub mod channel;
pub mod prepare;
pub mod queue;
pub mod scheduler;
pub mod selection;
pub mod sse;
pub mod state;

pub use cache::ProxyCache;
pub use channel::ChannelInfo;
pub use queue::RequestQueue;
pub use selection::{GroupInfo, GroupItemInfo, SelectionResult};

use self::state::LoadBalancerState;
use crate::api::handlers::admin::channels::{
    CustomHeader, EndpointConfig, EndpointType, UpstreamApiKey, parse_api_keys,
};
use crate::proxy::prepare::{
    extract_request_text, extract_response_text, extract_usage, estimate_tokens,
    prepare_proxy_request, select_channel_for_proxy,
};
use crate::protocol::inbound::Inbound;
use crate::protocol::outbound::Outbound;
use crate::proxy::sse::{
    apply_sse_usage, collect_sse_content, extract_error_from_sse, extract_usage_from_sse,
    find_sse_boundary, format_stream_error_event, sanitize_upstream_error,
};
use crate::stats::model::ModelRegistry;
use crate::stats::recorder::StatsRecorder;
use axum::body::Bytes;
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

    /// 选择渠道和端点（支持排除已失败渠道，精确匹配端点类型）
    pub async fn select_channel_with_exclude(
        &self,
        model: &str,
        endpoint_type: EndpointType,
        session_hash: Option<&str>,
        exclude_ids: &[String],
    ) -> Result<SelectionResult, ProxyError> {
        self.select_channel_inner(model, session_hash, exclude_ids, |ch| {
            ch.find_endpoint(&endpoint_type)
        })
        .await
    }

    /// 按模型选择渠道（不限端点类型，用于跨协议转换）
    pub async fn select_channel_for_model_with_exclude(
        &self,
        model: &str,
        session_hash: Option<&str>,
        exclude_ids: &[String],
    ) -> Result<SelectionResult, ProxyError> {
        self.select_channel_inner(model, session_hash, exclude_ids, |ch| {
            ch.endpoints.first().cloned()
        })
        .await
    }

    /// 选择渠道内部实现（统一逻辑）
    async fn select_channel_inner(
        &self,
        model: &str,
        session_hash: Option<&str>,
        exclude_ids: &[String],
        find_endpoint: impl Fn(&ChannelInfo) -> Option<EndpointConfig>,
    ) -> Result<SelectionResult, ProxyError> {
        // 1. 检查粘性会话
        if let Some(hash) = session_hash
            && let Some(channel_id) = self.lb_state.get_sticky_session(hash).await
            && !exclude_ids.contains(&channel_id)
            && let Ok(channel) = self.get_channel(&channel_id).await
            && let Some(endpoint) = find_endpoint(&channel)
        {
            let target_model = self.apply_model_mapping(&channel, model);
            return Ok(SelectionResult {
                channel,
                target_model,
                endpoint,
                group_id: None,
            });
        }

        // 2. 查找分组（精确匹配 → 正则匹配）
        let group = match self.find_group_by_name(model).await? {
            Some(g) => Some(g),
            None => self.find_group_by_regex(model).await?,
        };

        // 3. 从分组中选择渠道
        if let Some(group) = group
            && let Ok(item) = self.select_group_item(&group, exclude_ids).await
        {
            let channel = self.get_channel(&item.channel_id).await?;
            if let Some(endpoint) = find_endpoint(&channel) {
                if let Some(hash) = session_hash {
                    self.lb_state.set_sticky_session(hash, &channel.id).await;
                }
                let target_model = item.model_name.clone();
                return Ok(SelectionResult {
                    channel,
                    target_model,
                    endpoint,
                    group_id: Some(group.id),
                });
            }
        }

        // 4. 直接查找渠道
        let channel = self
            .find_channel_by_model(model, exclude_ids, |ch| find_endpoint(ch).is_some())
            .await?;
        if let Some(endpoint) = find_endpoint(&channel) {
            if let Some(hash) = session_hash {
                self.lb_state.set_sticky_session(hash, &channel.id).await;
            }
            let target_model = self.apply_model_mapping(&channel, model);
            return Ok(SelectionResult {
                channel,
                target_model,
                endpoint,
                group_id: None,
            });
        }

        Err(ProxyError::NoAvailableChannel("没有可用渠道".to_string()))
    }

    /// 根据名称查找分组
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
            "SELECT channel_id, model_name, priority, weight FROM group_items WHERE group_id = ? ORDER BY priority DESC, weight DESC"
        )
        .bind(group_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        Ok(items
            .into_iter()
            .map(|(channel_id, model_name, _priority, weight)| GroupItemInfo {
                channel_id,
                model_name,
                weight,
            })
            .collect())
    }

    /// 从分组中选择一个渠道项（自适应负载均衡，支持排除）
    async fn select_group_item(
        &self,
        group: &GroupInfo,
        exclude_ids: &[String],
    ) -> Result<GroupItemInfo, ProxyError> {
        if group.items.is_empty() {
            return Err(ProxyError::NoAvailableChannel(
                "分组没有可用渠道".to_string(),
            ));
        }

        // 计算每个渠道的评分（排除已失败渠道）
        let mut scored_items: Vec<(f64, &GroupItemInfo)> = Vec::new();

        for item in &group.items {
            if exclude_ids.contains(&item.channel_id) {
                continue;
            }
            let score = self
                .lb_state
                .calculate_score(&item.channel_id, item.weight)
                .await;
            if score > 0.0 {
                scored_items.push((score, item));
            }
        }

        if scored_items.is_empty() {
            return Err(ProxyError::NoAvailableChannel(
                "所有渠道都不可用".to_string(),
            ));
        }

        // 按评分排序
        scored_items.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        // Top-K 加权随机选择（K=3）
        let top_k = 3.min(scored_items.len());
        let top_items = &scored_items[..top_k];

        // 加权随机选择
        let total_score: f64 = top_items.iter().map(|(score, _)| score).sum();
        if total_score <= 0.0 {
            return Ok(top_items[0].1.clone());
        }

        use rand::Rng;
        let mut rng = rand::rng();
        let mut random_value = rng.random_range(0.0..total_score);

        for (score, item) in top_items {
            random_value -= score;
            if random_value <= 0.0 {
                return Ok((*item).clone());
            }
        }

        Ok(top_items[0].1.clone())
    }

    /// 获取渠道信息（带缓存）
    async fn get_channel(&self, channel_id: &str) -> Result<ChannelInfo, ProxyError> {
        // 1. 检查缓存
        if let Some(channel) = self.cache.get_channel(channel_id).await {
            return Ok(channel);
        }

        // 2. 缓存未命中，查询数据库
        let result = sqlx::query_as::<_, (String, String, String, String, String, String)>(
            "SELECT id, name, api_keys, endpoints, models, custom_headers FROM channels WHERE id = ? AND enabled = 1"
        )
        .bind(channel_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        let (id, name, api_keys_str, endpoints_str, models_str, custom_headers_str) =
            result.ok_or_else(|| ProxyError::ChannelNotFound("渠道不存在或已禁用".to_string()))?;

        let api_keys: Vec<UpstreamApiKey> = parse_api_keys(&api_keys_str);
        let endpoints: Vec<EndpointConfig> =
            serde_json::from_str(&endpoints_str).unwrap_or_default();
        let models = parse_models(&models_str);
        let custom_headers: Vec<CustomHeader> =
            serde_json::from_str(&custom_headers_str).unwrap_or_default();

        let channel = ChannelInfo {
            id,
            name,
            api_keys,
            endpoints,
            models,
            custom_headers,
        };

        // 3. 写入缓存
        self.cache.set_channel(channel.clone()).await;

        Ok(channel)
    }

    /// 按模型查找渠道（优先缓存索引，回退全表扫描）
    async fn find_channel_by_model(
        &self,
        model: &str,
        exclude_ids: &[String],
        endpoint_filter: impl Fn(&ChannelInfo) -> bool,
    ) -> Result<ChannelInfo, ProxyError> {
        // 1. 从 model_index 缓存查找
        let cached_ids = self.cache.find_channels_by_model(model).await;
        if !cached_ids.is_empty() {
            for cid in &cached_ids {
                if exclude_ids.contains(cid) {
                    continue;
                }
                if let Ok(channel) = self.get_channel(cid).await
                    && endpoint_filter(&channel)
                {
                    return Ok(channel);
                }
            }
        }

        // 2. 回退到数据库全表扫描（冷启动或缓存未命中）
        let channels = sqlx::query_as::<_, (String, String, String, String, String, String)>(
            "SELECT id, name, api_keys, endpoints, models, custom_headers FROM channels WHERE enabled = 1",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        for (id, name, api_keys_str, endpoints_str, models_str, custom_headers_str) in channels {
            if exclude_ids.contains(&id) {
                continue;
            }

            let models = parse_models(&models_str);
            if !models.iter().any(|m| m == model) {
                continue;
            }

            let api_keys: Vec<UpstreamApiKey> = parse_api_keys(&api_keys_str);
            let endpoints: Vec<EndpointConfig> =
                serde_json::from_str(&endpoints_str).unwrap_or_default();

            if endpoints.is_empty() {
                continue;
            }

            let custom_headers: Vec<CustomHeader> =
                serde_json::from_str(&custom_headers_str).unwrap_or_default();

            let channel = ChannelInfo {
                id: id.clone(),
                name: name.clone(),
                api_keys,
                endpoints,
                models,
                custom_headers,
            };

            if !endpoint_filter(&channel) {
                continue;
            }

            // 写入缓存供后续请求使用
            self.cache.set_channel(channel.clone()).await;
            return Ok(channel);
        }

        Err(ProxyError::NoAvailableChannel("没有可用渠道".to_string()))
    }

    /// 应用模型映射（模型映射已移至分组层，此处直接返回原始模型名）
    fn apply_model_mapping(&self, _channel: &ChannelInfo, model: &str) -> String {
        model.to_string()
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

/// 获取入站转换器（静态引用，避免堆分配）
pub fn get_inbound(endpoint_type: &EndpointType) -> &'static dyn Inbound {
    match endpoint_type {
        EndpointType::OpenAiChat => &crate::protocol::openai_chat::OpenAiChatInbound,
        EndpointType::OpenAiResponse => &crate::protocol::openai_responses::OpenAiResponsesInbound,
        EndpointType::Anthropic => &crate::protocol::anthropic::AnthropicInbound,
        _ => &crate::protocol::openai_chat::OpenAiChatInbound,
    }
}

/// 获取出站转换器（静态引用，避免堆分配）
pub fn get_outbound(endpoint_type: &EndpointType) -> &'static dyn Outbound {
    match endpoint_type {
        EndpointType::OpenAiChat => &crate::protocol::openai_chat::OpenAiChatOutbound,
        EndpointType::OpenAiResponse => &crate::protocol::openai_responses::OpenAiResponsesOutbound,
        EndpointType::Anthropic => &crate::protocol::anthropic::AnthropicOutbound,
        _ => &crate::protocol::openai_chat::OpenAiChatOutbound,
    }
}

/// 单次尝试的统计信息
struct AttemptStats {
    channel_id: String,
    target_model: String,
    upstream_endpoint: EndpointType,
    needs_conversion: bool,
    latency_ms: i64,
    status_code: u16,
    input_tokens: i32,
    output_tokens: i32,
    cache_read: i32,
    cache_creation: i32,
    cost: Option<f64>,
    error_message: Option<String>,
    upstream_key_hint: String,
}

impl crate::stats::recorder::RequestRecord {
    /// 从最后一次尝试构造完整记录
    #[allow(clippy::too_many_arguments)]
    fn from_last_attempt(
        last: &AttemptStats,
        api_key_id: Option<&str>,
        group_id: Option<&str>,
        model: &str,
        request_content: Option<String>,
        response_content: Option<String>,
        channel_attempts: Vec<crate::stats::recorder::ChannelAttempt>,
        ttft_ms: Option<i32>,
        is_stream: bool,
        user_agent: Option<String>,
    ) -> Self {
        Self {
            api_key_id: api_key_id.map(str::to_string),
            channel_id: Some(last.channel_id.clone()),
            group_id: group_id.map(str::to_string),
            requested_model: model.to_string(),
            actual_model: Some(last.target_model.clone()),
            input_tokens: last.input_tokens,
            output_tokens: last.output_tokens,
            cache_read_tokens: last.cache_read,
            cache_creation_tokens: last.cache_creation,
            cost: last.cost,
            latency_ms: Some(last.latency_ms as i32),
            ttft_ms,
            status_code: Some(last.status_code as i32),
            error_message: last.error_message.clone(),
            endpoint_type: Some(last.upstream_endpoint.as_str().to_string()),
            request_type: if last.needs_conversion {
                "conversion".to_string()
            } else {
                "passthrough".to_string()
            },
            request_content,
            response_content,
            is_stream,
            upstream_key_hint: Some(last.upstream_key_hint.clone()),
            attempts: channel_attempts,
            user_agent,
        }
    }

    /// 构造选择阶段失败时的最小记录（503 + "请求未到达上游"）
    #[allow(clippy::too_many_arguments)]
    fn minimal_for_select_failure(
        api_key_id: Option<&str>,
        group_id: Option<&str>,
        model: &str,
        request_content: Option<String>,
        response_content: Option<String>,
        channel_attempts: Vec<crate::stats::recorder::ChannelAttempt>,
        is_stream: bool,
        user_agent: Option<String>,
    ) -> Self {
        Self {
            api_key_id: api_key_id.map(str::to_string),
            channel_id: None,
            group_id: group_id.map(str::to_string),
            requested_model: model.to_string(),
            actual_model: None,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            cost: None,
            latency_ms: None,
            ttft_ms: None,
            status_code: Some(503),
            error_message: Some("请求未到达上游".to_string()),
            endpoint_type: None,
            request_type: "unknown".to_string(),
            request_content,
            response_content,
            is_stream,
            upstream_key_hint: None,
            attempts: channel_attempts,
            user_agent,
        }
    }
}

/// 保存单条请求日志（汇总所有尝试）
#[allow(clippy::too_many_arguments)]
async fn save_request_record(
    state: &ProxyState,
    api_key_id: Option<&str>,
    group_id: Option<&str>,
    model: &str,
    request_content: Option<String>,
    response_content: Option<String>,
    attempts: &[AttemptStats],
    ttft_ms: Option<i32>,
    is_stream: bool,
    user_agent: Option<String>,
) {
    // 构造 attempts 快照（用于记录日志）
    let channel_attempts: Vec<crate::stats::recorder::ChannelAttempt> = attempts
        .iter()
        .map(|a| crate::stats::recorder::ChannelAttempt {
            channel_id: a.channel_id.clone(),
            channel_name: None,
            status: if (200..400).contains(&a.status_code) {
                "success".to_string()
            } else {
                "failed".to_string()
            },
            duration_ms: a.latency_ms,
            error: a.error_message.clone(),
            upstream_key_hint: Some(a.upstream_key_hint.clone()),
        })
        .collect();

    let record = match attempts.last() {
        Some(last) => crate::stats::recorder::RequestRecord::from_last_attempt(
            last,
            api_key_id,
            group_id,
            model,
            request_content,
            response_content,
            channel_attempts,
            ttft_ms,
            is_stream,
            user_agent,
        ),
        None => crate::stats::recorder::RequestRecord::minimal_for_select_failure(
            api_key_id,
            group_id,
            model,
            request_content,
            response_content,
            channel_attempts,
            is_stream,
            user_agent,
        ),
    };

    let _ = state.stats_recorder.record_request(record).await;
}

/// 执行单次代理请求
#[allow(clippy::too_many_arguments)]
async fn execute_proxy_request(
    state: &ProxyState,
    _api_key_id: Option<&str>,
    upstream_api_key: &str,
    upstream_key_hint: &str,
    headers: &HeaderMap,
    body: &serde_json::Value,
    client_endpoint: &EndpointType,
    selection: &SelectionResult,
    attempts: &mut Vec<AttemptStats>,
) -> Result<ProxySuccess, ProxyError> {
    let prepared =
        prepare_proxy_request(headers, body, client_endpoint, selection, upstream_api_key).await?;
    let start_time = std::time::Instant::now();

    let response = state
        .http_client
        .post(&prepared.url)
        .headers(prepared.headers)
        .body(prepared.body)
        .send()
        .await
        .map_err(|e| ProxyError::RequestError(e.to_string()))?;

    let latency_ms = start_time.elapsed().as_millis() as i64;
    let status = response.status();
    let response_body = response.text().await.unwrap_or_default();

    let body_value: serde_json::Value = serde_json::from_str(&response_body).unwrap_or_default();
    let status_u16 = status.as_u16();

    let (input_tokens, output_tokens, cache_read, cache_creation) =
        if (200..400).contains(&status_u16) {
            let (i, o, cr, cc) = extract_usage(&body_value, &prepared.upstream_endpoint);
            if i == 0 && o == 0 {
                let req_text = extract_request_text(body);
                let resp_text = extract_response_text(&body_value);
                (
                    estimate_tokens(&req_text),
                    estimate_tokens(&resp_text),
                    cr,
                    cc,
                )
            } else {
                (i, o, cr, cc)
            }
        } else {
            (0, 0, 0, 0)
        };
    let cost = if input_tokens > 0 || output_tokens > 0 {
        Some(
            state
                .model_registry
                .calculate_cost(
                    &prepared.target_model,
                    input_tokens,
                    output_tokens,
                    cache_read,
                    cache_creation,
                )
                .await,
        )
    } else {
        None
    };

    attempts.push(AttemptStats {
        channel_id: prepared.channel_id.clone(),
        target_model: prepared.target_model.clone(),
        upstream_endpoint: prepared.upstream_endpoint.clone(),
        needs_conversion: prepared.needs_conversion,
        latency_ms,
        status_code: status_u16,
        input_tokens,
        output_tokens,
        cache_read,
        cache_creation,
        cost,
        error_message: if !status.is_success() {
            Some(response_body[..response_body.len().min(500)].to_string())
        } else {
            None
        },
        upstream_key_hint: upstream_key_hint.to_string(),
    });

    if !status.is_success() {
        tracing::warn!(
            "Upstream error: channel={}, status={}, body={}",
            prepared.channel_id,
            status,
            &response_body[..response_body.len().min(300)]
        );
        return Err(ProxyError::UpstreamError {
            status,
            body: response_body,
        });
    }

    state
        .lb_state
        .record_success(&prepared.channel_id, latency_ms as f64)
        .await;

    let final_body = if prepared.needs_conversion {
        let inbound = get_inbound(client_endpoint);
        let outbound = get_outbound(&prepared.upstream_endpoint);
        let llm_response = outbound
            .transform_response(response_body.as_bytes(), status.as_u16())
            .await
            .map_err(|e| ProxyError::TransformError(e.to_string()))?;
        inbound
            .transform_response(&llm_response)
            .map_err(|e| ProxyError::TransformError(e.to_string()))?
    } else {
        response_body.into_bytes()
    };

    Ok(ProxySuccess {
        status,
        body: final_body,
    })
}

/// 非流式代理请求（支持重试和排队）
pub async fn proxy_request(
    state: &ProxyState,
    api_key_id: Option<&str>,
    headers: &HeaderMap,
    body: &serde_json::Value,
    client_endpoint: &EndpointType,
) -> Result<ProxySuccess, ProxyError> {
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
    let request_content = serde_json::to_string(&body).ok();
    let user_agent = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let max_retries = 3;
    let mut exclude_ids = Vec::new();
    let mut last_error = None;
    let mut attempts = Vec::new();

    for attempt in 0..max_retries {
        let selection = match select_channel_for_proxy(
            state,
            headers,
            body,
            client_endpoint,
            &exclude_ids,
        )
        .await
        {
            Ok(s) => s,
            Err(e) => {
                // 渠道选择失败时也记录日志
                save_request_record(
                    state,
                    api_key_id,
                    None,
                    &model,
                    request_content.clone(),
                    None,
                    &attempts,
                    None,
                    false,
                    user_agent.clone(),
                )
                .await;
                return Err(e);
            }
        };
        let channel_id = selection.channel.id.clone();
        let group_id = selection.group_id.clone();
        let api_key_attempts = state.api_key_attempts(&selection.channel);

        for (key_idx, upstream_api_key) in api_key_attempts.iter().enumerate() {
            let key_hint = selection.channel.key_hint(upstream_api_key);
            match execute_proxy_request(
                state,
                api_key_id,
                upstream_api_key,
                &key_hint,
                headers,
                body,
                client_endpoint,
                &selection,
                &mut attempts,
            )
            .await
            {
                Ok(result) => {
                    save_request_record(
                        state,
                        api_key_id,
                        group_id.as_deref(),
                        &model,
                        request_content.clone(),
                        Some(String::from_utf8_lossy(&result.body).to_string()),
                        &attempts,
                        None,
                        false,
                        user_agent.clone(),
                    )
                    .await;
                    return Ok(result);
                }
                Err(ProxyError::UpstreamError { status, body }) => {
                    let can_try_next_key = key_idx + 1 < api_key_attempts.len()
                        && is_key_retryable_upstream_error(status, &body);

                    if can_try_next_key {
                        tracing::warn!(
                            "请求失败(第{}次), channel={}, status={}, 尝试同渠道下一个 key",
                            attempt + 1,
                            channel_id,
                            status
                        );
                        last_error = Some(ProxyError::UpstreamError { status, body });
                        continue;
                    }

                    tracing::warn!(
                        "请求失败(第{}次), channel={}, status={}, 排除后重试",
                        attempt + 1,
                        channel_id,
                        status
                    );
                    state
                        .lb_state
                        .record_failure(&channel_id, status.is_server_error())
                        .await;
                    exclude_ids.push(channel_id);
                    last_error = Some(ProxyError::UpstreamError { status, body });
                    break;
                }
                Err(e) => {
                    save_request_record(
                        state,
                        api_key_id,
                        group_id.as_deref(),
                        &model,
                        request_content.clone(),
                        None,
                        &attempts,
                        None,
                        false,
                        user_agent.clone(),
                    )
                    .await;
                    return Err(e);
                }
            }
        }
    }

    tracing::error!("所有重试耗尽, model={}", model);
    save_request_record(
        state,
        api_key_id,
        None,
        &model,
        request_content,
        None,
        &attempts,
        None,
        false,
        user_agent,
    )
    .await;
    Err(last_error
        .unwrap_or_else(|| ProxyError::NoAvailableChannel("所有渠道都不可用".to_string())))
}

/// 执行单次流式代理请求
#[allow(clippy::too_many_arguments)]
async fn execute_proxy_stream(
    state: &ProxyState,
    api_key_id: Option<&str>,
    upstream_api_key: &str,
    upstream_key_hint: String,
    group_id: Option<String>,
    headers: &HeaderMap,
    body: &serde_json::Value,
    client_endpoint: &EndpointType,
    selection: &SelectionResult,
    attempts: &mut Vec<AttemptStats>,
    _permit: Option<tokio::sync::OwnedSemaphorePermit>,
) -> Result<
    (
        StatusCode,
        std::pin::Pin<
            Box<
                dyn futures::Stream<Item = Result<Bytes, std::convert::Infallible>>
                    + Send
                    + 'static,
            >,
        >,
        String,
        Option<i32>,
    ),
    ProxyError,
> {
    let prepared =
        prepare_proxy_request(headers, body, client_endpoint, selection, upstream_api_key).await?;
    let start_time = std::time::Instant::now();

    let response = state
        .http_client
        .post(&prepared.url)
        .headers(prepared.headers)
        .body(prepared.body)
        .send()
        .await
        .map_err(|e| ProxyError::RequestError(e.to_string()))?;

    if !response.status().is_success() {
        let latency_ms = start_time.elapsed().as_millis() as i64;
        let status = response.status();
        let response_body = response.text().await.unwrap_or_default();

        attempts.push(AttemptStats {
            channel_id: prepared.channel_id.clone(),
            target_model: prepared.target_model.clone(),
            upstream_endpoint: prepared.upstream_endpoint.clone(),
            needs_conversion: prepared.needs_conversion,
            latency_ms,
            status_code: status.as_u16(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read: 0,
            cache_creation: 0,
            cost: None,
            error_message: Some(response_body[..response_body.len().min(500)].to_string()),
            upstream_key_hint: upstream_key_hint.clone(),
        });

        return Err(ProxyError::UpstreamError {
            status,
            body: response_body,
        });
    }

    use futures::StreamExt;
    let mut upstream_stream = response.bytes_stream();

    let mut initial_buffer = Vec::new();
    while let Some(chunk) = upstream_stream.next().await {
        match chunk {
            Ok(bytes) => {
                initial_buffer.extend_from_slice(&bytes);
                if find_sse_boundary(&initial_buffer).is_some() || initial_buffer.len() >= 64 * 1024
                {
                    break;
                }
            }
            Err(e) => return Err(ProxyError::RequestError(e.to_string())),
        }
    }

    if let Some(event_end) = find_sse_boundary(&initial_buffer)
        && let Ok(text) = std::str::from_utf8(&initial_buffer[..event_end])
        && let Some(error) = extract_error_from_sse(text, &prepared.upstream_endpoint)
    {
        let latency_ms = start_time.elapsed().as_millis() as i64;
        let sanitized_error = sanitize_upstream_error(&error);

        attempts.push(AttemptStats {
            channel_id: prepared.channel_id.clone(),
            target_model: prepared.target_model.clone(),
            upstream_endpoint: prepared.upstream_endpoint.clone(),
            needs_conversion: prepared.needs_conversion,
            latency_ms,
            status_code: StatusCode::BAD_GATEWAY.as_u16(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read: 0,
            cache_creation: 0,
            cost: None,
            error_message: Some(sanitized_error),
            upstream_key_hint: upstream_key_hint.clone(),
        });

        return Err(ProxyError::UpstreamError {
            status: StatusCode::BAD_GATEWAY,
            body: error,
        });
    }

    let upstream_stream = futures::stream::iter(
        (!initial_buffer.is_empty())
            .then(|| Ok::<Bytes, reqwest::Error>(Bytes::from(initial_buffer))),
    )
    .chain(upstream_stream);

    let state_clone = state.clone();
    let channel_id_clone = prepared.channel_id.clone();
    let model_clone = prepared.model.clone();
    let target_model_clone = prepared.target_model.clone();
    let upstream_endpoint_clone = prepared.upstream_endpoint.clone();
    let client_endpoint_clone = client_endpoint.clone();
    let needs_conversion = prepared.needs_conversion;
    let api_key_id_clone = api_key_id.map(|s| s.to_string());
    let request_content_clone = serde_json::to_string(&body).ok();
    let req_text_for_estimation = extract_request_text(body);

    let (stats_tx, stats_rx) = tokio::sync::oneshot::channel::<(
        i32,            // status_code
        i32,            // input_tokens
        i32,            // output_tokens
        i32,            // cache_read
        i32,            // cache_creation
        Option<f64>,    // cost
        i32,            // latency_ms
        Option<String>, // error_message
        Option<String>, // response_content
        Option<i32>,    // ttft_ms
    )>();

    // 提前 clone 给 spawn 任务使用（async_stream 会 move 原值）
    let sc_channel_id = channel_id_clone.clone();
    let sc_model = model_clone.clone();
    let sc_target_model = target_model_clone.clone();
    let sc_client_endpoint = client_endpoint_clone.clone();
    let sc_needs_conversion = needs_conversion;
    let sc_api_key_id = api_key_id_clone.clone();
    let sc_request_content = request_content_clone.clone();
    let sc_upstream_key_hint = upstream_key_hint.clone();
    let sc_user_agent = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let stats_recorder = state.stats_recorder.clone();
    let attempts_snapshot: Vec<crate::stats::recorder::ChannelAttempt> = attempts
        .iter()
        .map(|a| crate::stats::recorder::ChannelAttempt {
            channel_id: a.channel_id.clone(),
            channel_name: None,
            status: if (200..400).contains(&a.status_code) {
                "success".to_string()
            } else {
                "failed".to_string()
            },
            duration_ms: a.latency_ms,
            error: a.error_message.clone(),
            upstream_key_hint: Some(a.upstream_key_hint.clone()),
        })
        .collect();

    let response_stream = async_stream::stream! {
        // _permit 随 stream 生命周期存在，stream drop 时释放 semaphore 许可
        let _permit = _permit;
        let mut stream = std::pin::pin!(upstream_stream);
        let mut last_usage: Option<serde_json::Value> = None;
        let mut input_usage: Option<serde_json::Value> = None;
        let mut buffer = Vec::new();
        let mut collected_text = String::new();
        let mut collected_reasoning = String::new();
        let mut collected_tool_calls: Vec<serde_json::Value> = Vec::new();
        let mut stream_error: Option<String> = None;
        let mut ttft_ms: Option<i32> = None;
        let mut first_token_seen = false;

        if needs_conversion {
            let inbound = get_inbound(&client_endpoint_clone);
            let outbound = get_outbound(&upstream_endpoint_clone);

            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        buffer.extend_from_slice(&bytes);

                        while let Some(event_end) = find_sse_boundary(&buffer) {
                            let event_bytes = buffer[..event_end].to_vec();
                            buffer = buffer[event_end..].to_vec();

                            if event_bytes.iter().all(|b| *b == b'\n' || *b == b'\r') {
                                continue;
                            }

                            if let Ok(text) = std::str::from_utf8(&event_bytes)
                                && let Some(source) = extract_usage_from_sse(text, &upstream_endpoint_clone) {
                                    apply_sse_usage(source, &mut last_usage, &mut input_usage);
                                }
                            let mut is_error_event = false;
                            if stream_error.is_none()
                                && let Ok(text) = std::str::from_utf8(&event_bytes)
                                && let Some(error) = extract_error_from_sse(text, &upstream_endpoint_clone) {
                                    stream_error = Some(error);
                                    is_error_event = true;
                                }
                            if is_error_event {
                                if let Some(error) = stream_error.as_deref() {
                                    yield Ok::<_, std::convert::Infallible>(Bytes::from(format_stream_error_event(
                                        error,
                                        &client_endpoint_clone,
                                    )));
                                }
                                continue;
                            }

                            if !first_token_seen {
                                ttft_ms = Some(start_time.elapsed().as_millis() as i32);
                                first_token_seen = true;
                            }

                            match outbound.transform_stream_event(&event_bytes) {
                                Ok(Some(llm_stream)) => {
                                    if let Some(choice) = llm_stream.first_choice() {
                                        if let Some(crate::protocol::model::Content::Text(t)) = &choice.delta.content
                                            && !t.is_empty() {
                                                collected_text.push_str(t);
                                            }
                                        if let Some(r) = &choice.delta.reasoning_content {
                                            collected_reasoning.push_str(r);
                                        }
                                        if let Some(tcs) = &choice.delta.tool_calls {
                                            for tc in tcs {
                                                collected_tool_calls.push(serde_json::json!({
                                                    "id": tc.id,
                                                    "name": tc.function.name,
                                                    "arguments": tc.function.arguments,
                                                }));
                                            }
                                        }
                                    }
                                    match inbound.transform_stream_event(&llm_stream) {
                                        Ok(converted) => {
                                            yield Ok::<_, std::convert::Infallible>(Bytes::from(converted));
                                        }
                                        Err(e) => {
                                            tracing::error!("Stream inbound conversion error: {}", e);
                                        }
                                    }
                                }
                                Ok(None) => {}
                                Err(e) => {
                                    tracing::error!("Stream outbound conversion error: {}", e);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Upstream stream error: {}", e);
                        break;
                    }
                }
            }

            if !buffer.is_empty() && !buffer.iter().all(|b| *b == b'\n' || *b == b'\r') {
                if let Ok(text) = std::str::from_utf8(&buffer)
                    && let Some(source) = extract_usage_from_sse(text, &upstream_endpoint_clone) {
                        apply_sse_usage(source, &mut last_usage, &mut input_usage);
                    }
                let mut is_error_event = false;
                if stream_error.is_none()
                    && let Ok(text) = std::str::from_utf8(&buffer)
                    && let Some(error) = extract_error_from_sse(text, &upstream_endpoint_clone) {
                        stream_error = Some(error);
                        is_error_event = true;
                    }
                if !is_error_event {
                    if let Ok(Some(llm_stream)) = outbound.transform_stream_event(&buffer)
                        && let Ok(converted) = inbound.transform_stream_event(&llm_stream) {
                            yield Ok(Bytes::from(converted));
                        }
                } else if let Some(error) = stream_error.as_deref() {
                    yield Ok(Bytes::from(format_stream_error_event(
                        error,
                        &client_endpoint_clone,
                    )));
                }
            }
        } else {
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        buffer.extend_from_slice(&bytes);

                        while let Some(event_end) = find_sse_boundary(&buffer) {
                            let event_bytes = buffer[..event_end].to_vec();
                            buffer = buffer[event_end..].to_vec();

                            if event_bytes.iter().all(|b| *b == b'\n' || *b == b'\r') {
                                continue;
                            }

                            if let Ok(text) = std::str::from_utf8(&event_bytes) {
                                if let Some(source) = extract_usage_from_sse(text, &upstream_endpoint_clone) {
                                    apply_sse_usage(source, &mut last_usage, &mut input_usage);
                                }
                                if stream_error.is_none()
                                    && let Some(error) = extract_error_from_sse(text, &upstream_endpoint_clone) {
                                    stream_error = Some(error);
                                }
                                collect_sse_content(text, &upstream_endpoint_clone, &mut collected_text, &mut collected_reasoning, &mut collected_tool_calls);
                            }
                        }

                        if !first_token_seen {
                            ttft_ms = Some(start_time.elapsed().as_millis() as i32);
                            first_token_seen = true;
                        }
                        yield Ok::<_, std::convert::Infallible>(bytes);
                    }
                    Err(e) => {
                        tracing::error!("Stream error: {}", e);
                        break;
                    }
                }
            }

            // 处理 buffer 中残余的最后一个事件
            if !buffer.is_empty() && !buffer.iter().all(|b| *b == b'\n' || *b == b'\r')
                && let Ok(text) = std::str::from_utf8(&buffer) {
                    if let Some(source) = extract_usage_from_sse(text, &upstream_endpoint_clone) {
                        apply_sse_usage(source, &mut last_usage, &mut input_usage);
                    }
                    if stream_error.is_none()
                        && let Some(error) = extract_error_from_sse(text, &upstream_endpoint_clone) {
                        stream_error = Some(error);
                    }
                    collect_sse_content(text, &upstream_endpoint_clone, &mut collected_text, &mut collected_reasoning, &mut collected_tool_calls);
                }
        }

        // 流结束后发送统计到 oneshot
        let latency_ms = start_time.elapsed().as_millis() as i64;
        let (mut input_tokens, mut output_tokens, cache_read, cache_creation) = match &upstream_endpoint_clone {
            EndpointType::Anthropic => {
                let input = input_usage.as_ref()
                    .and_then(|u| u["input_tokens"].as_i64())
                    .filter(|&v| v > 0)
                    .or_else(|| last_usage.as_ref().and_then(|u| u["usage"]["input_tokens"].as_i64()))
                    .unwrap_or(0) as i32;
                let output = last_usage.as_ref()
                    .and_then(|u| u["usage"]["output_tokens"].as_i64())
                    .unwrap_or(0) as i32;
                let cache_read = input_usage.as_ref()
                    .and_then(|u| u["cache_read_input_tokens"].as_i64())
                    .filter(|&v| v > 0)
                    .or_else(|| last_usage.as_ref().and_then(|u| u["usage"]["cache_read_input_tokens"].as_i64()))
                    .unwrap_or(0) as i32;
                let cache_creation = input_usage.as_ref()
                    .and_then(|u| u["cache_creation_input_tokens"].as_i64())
                    .filter(|&v| v > 0)
                    .or_else(|| last_usage.as_ref().and_then(|u| u["usage"]["cache_creation_input_tokens"].as_i64()))
                    .unwrap_or(0) as i32;
                (input, output, cache_read, cache_creation)
            }
            _ => {
                last_usage
                    .map(|u| extract_usage(&u, &upstream_endpoint_clone))
                    .unwrap_or((0, 0, 0, 0))
            }
        };

        // 兜底估算：上游不返回 usage 时从内容长度推算
        if input_tokens == 0 && output_tokens == 0 {
            input_tokens = estimate_tokens(&req_text_for_estimation);
            output_tokens = estimate_tokens(&collected_text);
        }

        let cost = if input_tokens > 0 || output_tokens > 0 {
            Some(state_clone.model_registry.calculate_cost(
                &target_model_clone,
                input_tokens,
                output_tokens,
                cache_read,
                cache_creation,
            ).await)
        } else {
            None
        };

        let (status_code, error_message, response_content) = if let Some(error) = stream_error {
            state_clone.lb_state.record_failure(&channel_id_clone, false).await;
            (
                502i32,
                Some(sanitize_upstream_error(&error)),
                Some(error),
            )
        } else {
            state_clone.lb_state.record_success(&channel_id_clone, latency_ms as f64).await;
            let resp = if collected_text.is_empty() && collected_reasoning.is_empty()
                && collected_tool_calls.is_empty() && input_tokens == 0 && output_tokens == 0
            {
                None
            } else {
                let mut resp_json = serde_json::json!({
                    "content": collected_text,
                    "usage": {
                        "input_tokens": input_tokens,
                        "output_tokens": output_tokens,
                        "cache_read_tokens": cache_read,
                        "cache_creation_tokens": cache_creation,
                    }
                });
                if !collected_reasoning.is_empty() {
                    resp_json["reasoning"] = serde_json::json!(collected_reasoning);
                }
                if !collected_tool_calls.is_empty() {
                    resp_json["tool_calls"] = serde_json::json!(collected_tool_calls);
                }
                Some(resp_json.to_string())
            };
            (200i32, None, resp)
        };

        let _ = stats_tx.send((
            status_code,
            input_tokens,
            output_tokens,
            cache_read,
            cache_creation,
            cost,
            latency_ms as i32,
            error_message,
            response_content,
            ttft_ms,
        ));
    };

    // 后台任务确保统计写入（即使流被 drop 也能通过 rx 检测到）
    tokio::spawn(async move {
        let result = match stats_rx.await {
            Ok((
                status_code,
                input_tokens,
                output_tokens,
                cache_read,
                cache_creation,
                cost,
                latency_ms,
                error_message,
                response_content,
                ttft_ms,
            )) => {
                let mut channel_attempts = attempts_snapshot;
                channel_attempts.push(crate::stats::recorder::ChannelAttempt {
                    channel_id: sc_channel_id.clone(),
                    channel_name: None,
                    status: if (200..400).contains(&status_code) {
                        "success".to_string()
                    } else {
                        "failed".to_string()
                    },
                    duration_ms: latency_ms as i64,
                    error: error_message.clone(),
                    upstream_key_hint: Some(sc_upstream_key_hint.clone()),
                });

                let record = crate::stats::recorder::RequestRecord {
                    api_key_id: sc_api_key_id,
                    channel_id: Some(sc_channel_id),
                    group_id,
                    requested_model: sc_model,
                    actual_model: Some(sc_target_model),
                    input_tokens,
                    output_tokens,
                    cache_read_tokens: cache_read,
                    cache_creation_tokens: cache_creation,
                    cost,
                    latency_ms: Some(latency_ms),
                    ttft_ms,
                    status_code: Some(status_code),
                    error_message,
                    endpoint_type: Some(sc_client_endpoint.as_str().to_string()),
                    request_type: if sc_needs_conversion {
                        "conversion".to_string()
                    } else {
                        "passthrough".to_string()
                    },
                    request_content: sc_request_content,
                    response_content,
                    is_stream: true,
                    upstream_key_hint: Some(sc_upstream_key_hint),
                    attempts: channel_attempts,
                    user_agent: sc_user_agent,
                };
                stats_recorder.record_request(record).await
            }
            Err(_) => {
                tracing::warn!("Stream dropped before completion, stats may be partial");
                Ok(())
            }
        };
        if let Err(e) = result {
            tracing::warn!("Failed to save stream stats: {}", e);
        }
    });

    Ok((
        StatusCode::OK,
        Box::pin(response_stream),
        "text/event-stream".to_string(),
        None,
    ))
}

/// 流式代理请求（支持重试和排队）
pub async fn proxy_stream(
    state: &ProxyState,
    api_key_id: Option<&str>,
    headers: &HeaderMap,
    body: &serde_json::Value,
    client_endpoint: &EndpointType,
) -> Result<
    (
        StatusCode,
        std::pin::Pin<
            Box<
                dyn futures::Stream<Item = Result<Bytes, std::convert::Infallible>>
                    + Send
                    + 'static,
            >,
        >,
        String,
    ),
    ProxyError,
> {
    let mut permit = if let Some(queue) = &state.queue {
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
    let max_retries = 3;
    let mut exclude_ids = Vec::new();
    let mut last_error = None;
    let mut attempts = Vec::new();

    for attempt in 0..max_retries {
        let selection = match select_channel_for_proxy(
            state,
            headers,
            body,
            client_endpoint,
            &exclude_ids,
        )
        .await
        {
            Ok(s) => s,
            Err(e) => {
                // 渠道选择失败时也记录日志
                save_request_record(
                    state,
                    api_key_id,
                    None,
                    &model,
                    request_content.clone(),
                    None,
                    &attempts,
                    None,
                    true,
                    user_agent.clone(),
                )
                .await;
                return Err(e);
            }
        };
        let channel_id = selection.channel.id.clone();
        let group_id = selection.group_id.clone();
        let api_key_attempts = state.api_key_attempts(&selection.channel);

        for (key_idx, upstream_api_key) in api_key_attempts.iter().enumerate() {
            let key_hint = selection.channel.key_hint(upstream_api_key);
            match execute_proxy_stream(
                state,
                api_key_id,
                upstream_api_key,
                key_hint,
                group_id.clone(),
                headers,
                body,
                client_endpoint,
                &selection,
                &mut attempts,
                permit.take(),
            )
            .await
            {
                Ok((status, stream, content_type, _ttft)) => {
                    return Ok((status, stream, content_type));
                }
                Err(ProxyError::UpstreamError { status, body }) => {
                    let can_try_next_key = key_idx + 1 < api_key_attempts.len()
                        && is_key_retryable_upstream_error(status, &body);

                    if can_try_next_key {
                        tracing::warn!(
                            "流式请求失败(第{}次), channel={}, status={}, 尝试同渠道下一个 key",
                            attempt + 1,
                            channel_id,
                            status
                        );
                        last_error = Some(ProxyError::UpstreamError { status, body });
                        continue;
                    }

                    tracing::warn!(
                        "流式请求失败(第{}次), channel={}, status={}, 排除后重试",
                        attempt + 1,
                        channel_id,
                        status
                    );
                    state
                        .lb_state
                        .record_failure(&channel_id, status.is_server_error())
                        .await;
                    exclude_ids.push(channel_id);
                    last_error = Some(ProxyError::UpstreamError { status, body });
                    break;
                }
                Err(e) => {
                    save_request_record(
                        state,
                        api_key_id,
                        group_id.as_deref(),
                        &model,
                        request_content.clone(),
                        None,
                        &attempts,
                        None,
                        true,
                        user_agent.clone(),
                    )
                    .await;
                    return Err(e);
                }
            }
        }
    }

    tracing::error!("流式重试耗尽, model={}", model);
    save_request_record(
        state,
        api_key_id,
        None,
        &model,
        request_content,
        None,
        &attempts,
        None,
        true,
        user_agent,
    )
    .await;
    Err(last_error
        .unwrap_or_else(|| ProxyError::NoAvailableChannel("所有渠道都不可用".to_string())))
}

/// 错误格式类型
pub enum ErrorFormat {
    /// OpenAI 格式: {"error": {"message": ..., "type": ...}}
    OpenAi,
    /// Anthropic 格式: {"type": "error", "error": {"type": ..., "message": ...}}
    Anthropic,
}

/// 统一代理请求入口（供各 handler 调用）
pub async fn handle_proxy_request(
    state: &ProxyState,
    auth: crate::api::middleware::ApiKeyAuth,
    headers: HeaderMap,
    body: serde_json::Value,
    client_endpoint: &crate::api::handlers::admin::channels::EndpointType,
    error_format: &ErrorFormat,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let model = body["model"].as_str().unwrap_or("unknown");
    let is_stream = body["stream"].as_bool().unwrap_or(false);
    let api_key_id = Some(auth.key_id.as_str());

    // 验证 API Key 是否有权访问目标模型
    if let Err(e) = validate_model_access(&state.pool, &auth.key_id, model).await {
        return format_proxy_error(e, error_format);
    }

    if is_stream {
        match proxy_stream(state, api_key_id, &headers, &body, client_endpoint).await {
            Ok((status, stream, content_type)) => axum::response::Response::builder()
                .status(status)
                .header("Content-Type", content_type)
                .header("Cache-Control", "no-cache")
                .header("Connection", "keep-alive")
                .body(axum::body::Body::from_stream(stream))
                .unwrap()
                .into_response(),
            Err(e) => format_proxy_error(e, error_format),
        }
    } else {
        match proxy_request(state, api_key_id, &headers, &body, client_endpoint).await {
            Ok(result) => axum::response::Response::builder()
                .status(result.status)
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(result.body))
                .unwrap()
                .into_response(),
            Err(e) => format_proxy_error(e, error_format),
        }
    }
}

/// 验证 API Key 是否有权访问目标模型
async fn validate_model_access(
    pool: &SqlitePool,
    key_id: &str,
    model: &str,
) -> Result<(), ProxyError> {
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
        let allowed = crate::api::handlers::admin::api_keys::parse_supported_models(&models_str);
        if !allowed.iter().any(|m| m == model) {
            return Err(ProxyError::NoAvailableChannel(format!(
                "API Key 无权访问模型: {}",
                model
            )));
        }
    }

    Ok(())
}

/// 格式化代理错误为 HTTP 响应
pub fn format_proxy_error(e: ProxyError, format: &ErrorFormat) -> axum::response::Response {
    use axum::response::IntoResponse;

    match (e, format) {
        (ProxyError::NoAvailableChannel(msg), ErrorFormat::OpenAi) => (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({
                "error": { "message": msg, "type": "server_error" }
            })),
        )
            .into_response(),
        (ProxyError::NoAvailableChannel(msg), ErrorFormat::Anthropic) => (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({
                "type": "error",
                "error": { "type": "api_error", "message": msg }
            })),
        )
            .into_response(),
        (ProxyError::UpstreamError { status, body }, ErrorFormat::OpenAi) => {
            let msg = sanitize_upstream_error(&body);
            (
                status,
                axum::Json(serde_json::json!({
                    "error": { "message": msg, "type": "server_error" }
                })),
            )
                .into_response()
        }
        (ProxyError::UpstreamError { status, body }, ErrorFormat::Anthropic) => {
            let msg = sanitize_upstream_error(&body);
            (
                status,
                axum::Json(serde_json::json!({
                    "type": "error",
                    "error": { "type": "api_error", "message": msg }
                })),
            )
                .into_response()
        }
        (e, ErrorFormat::OpenAi) => (
            StatusCode::BAD_GATEWAY,
            axum::Json(serde_json::json!({
                "error": { "message": e.to_string(), "type": "server_error" }
            })),
        )
            .into_response(),
        (e, ErrorFormat::Anthropic) => (
            StatusCode::BAD_GATEWAY,
            axum::Json(serde_json::json!({
                "type": "error",
                "error": { "type": "api_error", "message": e.to_string() }
            })),
        )
            .into_response(),
    }
}

/// 验证字符串可作为 HTTP header value
/// 用于在保存上游 API Key 时一次性拦截含 CRLF / 控制字符的输入，
/// 避免转发时 `HeaderValue::from_str(...).unwrap()` panic。
pub(crate) fn validate_header_value(s: &str) -> Result<(), String> {
    reqwest::header::HeaderValue::from_str(s)
        .map(|_| ())
        .map_err(|e| format!("含非法 header 字符 ({e})"))
}

/// 兼容封装：仅根据 status + body 判断上游错误是否需要换 Key
///
/// 内部委托给 `ProxyError::is_key_retryable`，避免散落的多处独立判断。
fn is_key_retryable_upstream_error(status: StatusCode, body: &str) -> bool {
    ProxyError::UpstreamError {
        status,
        body: body.to_string(),
    }
    .is_key_retryable()
}

/// 解析 models 字段
fn parse_models(models_str: &str) -> Vec<String> {
    serde_json::from_str(models_str).unwrap_or_default()
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
            ProxyError::TransformError(_) => ErrorClass::Internal,
            ProxyError::UpstreamError { status, body } => classify_upstream(*status, body),
        }
    }

    /// 上游错误是否应当换 Key 重试
    pub fn is_key_retryable(&self) -> bool {
        matches!(self.classify(), ErrorClass::KeyRetryable)
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
