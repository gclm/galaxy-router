pub mod cache;
pub mod channel;
pub mod circuit;
pub mod execute;
pub mod prepare;
pub mod queue;
pub mod ratelimit;
pub mod scheduler;
pub mod selection;
pub mod sse;
pub mod state;

pub use cache::ProxyCache;
pub use channel::ChannelInfo;
pub use queue::RequestQueue;
pub use ratelimit::RateLimiter;
pub use selection::{GroupInfo, GroupItemInfo, SelectionResult};

use self::state::{ChannelLoadInfo, LoadBalancerState};
use crate::api::handlers::admin::channels::{
    CustomHeader, EndpointConfig, EndpointType, UpstreamApiKey, parse_api_keys,
};
use crate::protocol::inbound::Inbound;
use crate::protocol::outbound::Outbound;
use crate::proxy::execute::{execute_proxy_stream, proxy_request, save_request_record};
use crate::proxy::prepare::select_channel_for_proxy;
use crate::proxy::sse::sanitize_upstream_error;
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
            && self.lb_state.is_channel_available(&channel_id).await
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
        if let Some(ref group) = group {
            match self.select_group_item(group, exclude_ids).await {
                Ok(item) => {
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
                            group_id: Some(group.id.clone()),
                        });
                    }
                }
                Err(e) => {
                    // 分组存在但渠道不可用 → 503（不立即返回，先尝试直接渠道查找）
                    tracing::warn!("分组 {} 渠道选择失败: {}", group.name, e);
                }
            }
        }

        // 4. 直接查找渠道
        let channel_result = self
            .find_channel_by_model(model, exclude_ids, |ch| find_endpoint(ch).is_some())
            .await;
        if let Ok(channel) = channel_result
            && let Some(endpoint) = find_endpoint(&channel)
        {
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

        // 5. 所有路径都失败：区分模型不存在 vs 渠道不可用
        if group.is_some() {
            Err(ProxyError::NoAvailableChannel(format!(
                "模型 {} 的所有渠道不可用或被排除",
                model
            )))
        } else {
            Err(ProxyError::ModelNotFound(format!(
                "模型不存在: {}",
                model
            )))
        }
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
            "SELECT channel_id, model_name, priority, weight FROM group_items WHERE group_id = ? ORDER BY priority ASC, weight DESC"
        )
        .bind(group_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        Ok(items
            .into_iter()
            .map(
                |(channel_id, model_name, priority, weight)| GroupItemInfo {
                    channel_id,
                    model_name,
                    priority,
                    weight,
                },
            )
            .collect())
    }

    /// 从分组中选择一个渠道项（优先级分层 + 自适应负载均衡，支持排除）
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

        // 过滤掉排除的渠道和不可用渠道
        let mut available: Vec<&GroupItemInfo> = Vec::new();
        for item in &group.items {
            if exclude_ids.contains(&item.channel_id) {
                continue;
            }
            // 检查渠道可用性（熔断器）
            if !self.lb_state.is_channel_available(&item.channel_id).await {
                continue;
            }
            // 确保渠道状态已初始化
            let channel = self.get_channel(&item.channel_id).await?;
            self.lb_state.ensure_channel_status(&item.channel_id, channel.max_concurrency).await;
            available.push(item);
        }

        if available.is_empty() {
            return Err(ProxyError::NoAvailableChannel(
                "所有渠道不可用或被排除".to_string(),
            ));
        }

        // === 分层过滤选择（sub2api 策略）===
        // 检查是否有任何渠道配置了 max_concurrency
        let mut has_concurrency_config = false;
        for item in &available {
            if let Some(info) = self.lb_state.get_channel_load_info(&item.channel_id).await
                && info.max_concurrency > 0
            {
                has_concurrency_config = true;
                break;
            }
        }

        if has_concurrency_config {
            // 使用分层过滤：优先级 → 负载率 → LRU → 随机
            self.select_by_layered_filtering(available).await
        } else {
            // 无并发配置时回退到加权随机（保持原有行为）
            self.select_by_weighted_random(&available).await
        }
    }

    /// 分层过滤选择：优先级 → 负载率 → LRU → 随机
    async fn select_by_layered_filtering(
        &self,
        candidates: Vec<&GroupItemInfo>,
    ) -> Result<GroupItemInfo, ProxyError> {
        use rand::Rng;

        // 第一层：过滤出最高优先级（priority 数值最小）
        let min_priority = candidates.iter().map(|c| c.priority).min().unwrap_or(0);
        let tier: Vec<&GroupItemInfo> = candidates.into_iter()
            .filter(|c| c.priority == min_priority)
            .collect();

        // 收集负载信息
        let load_infos: Vec<Option<ChannelLoadInfo>> = {
            let mut infos = Vec::with_capacity(tier.len());
            for c in &tier {
                infos.push(self.lb_state.get_channel_load_info(&c.channel_id).await);
            }
            infos
        };

        // 第二层：过滤出最低负载率
        // 找到配置了 max_concurrency 的候选中最低的负载率
        let min_load_rate = tier.iter().enumerate()
            .filter_map(|(i, _)| {
                load_infos[i].as_ref()
                    .filter(|info| info.max_concurrency > 0)
                    .map(|info| info.load_rate)
            })
            .min()
            .unwrap_or(0);

        let filtered: Vec<usize> = tier.iter().enumerate()
            .filter(|(i, _)| {
                match &load_infos[*i] {
                    Some(info) if info.max_concurrency > 0 => info.load_rate <= min_load_rate,
                    _ => true, // 无并发限制的渠道始终保留
                }
            })
            .map(|(i, _)| i)
            .collect();

        if filtered.is_empty() {
            let item = tier.first().unwrap();
            return Ok((*item).clone());
        }

        // 第三层：LRU（最久未使用）
        let min_last_used = filtered.iter()
            .filter_map(|&i| load_infos[i].as_ref().map(|info| info.last_used_at))
            .min()
            .unwrap_or(std::time::Instant::now());

        let lru_indices: Vec<usize> = filtered.into_iter()
            .filter(|&i| {
                load_infos[i].as_ref()
                    .map(|info| info.last_used_at <= min_last_used + std::time::Duration::from_millis(1))
                    .unwrap_or(true)
            })
            .collect();

        if lru_indices.is_empty() {
            let mut rng = rand::rng();
            let idx = rng.random_range(0..tier.len());
            return Ok(tier[idx].clone());
        }

        // 第四层：随机打破平局
        let mut rng = rand::rng();
        let selected_idx = lru_indices[rng.random_range(0..lru_indices.len())];
        Ok(tier[selected_idx].clone())
    }

    /// 加权随机选择（原有策略，无并发配置时使用）
    async fn select_by_weighted_random(
        &self,
        candidates: &[&GroupItemInfo],
    ) -> Result<GroupItemInfo, ProxyError> {
        use rand::Rng;

        // 按优先级分组
        let mut priority_tiers: std::collections::BTreeMap<i32, Vec<&GroupItemInfo>> =
            std::collections::BTreeMap::new();
        for item in candidates {
            priority_tiers
                .entry(item.priority)
                .or_default()
                .push(item);
        }

        for tier_items in priority_tiers.values() {
            let mut scored_items: Vec<(f64, &GroupItemInfo)> = Vec::new();

            for item in tier_items {
                let score = self
                    .lb_state
                    .calculate_score(&item.channel_id, item.weight)
                    .await;
                if score > 0.0 {
                    scored_items.push((score, item));
                }
            }

            if scored_items.is_empty() {
                continue;
            }

            // 按评分排序
            scored_items.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

            // Top-K 加权随机选择（K=3）
            let top_k = 3.min(scored_items.len());
            let top_items = &scored_items[..top_k];

            let total_score: f64 = top_items.iter().map(|(score, _)| score).sum();
            if total_score <= 0.0 {
                return Ok(top_items[0].1.clone());
            }

            let mut rng = rand::rng();
            let mut random_value = rng.random_range(0.0..total_score);

            for (score, item) in top_items {
                random_value -= score;
                if random_value <= 0.0 {
                    return Ok((*item).clone());
                }
            }

            return Ok(top_items[0].1.clone());
        }

        Err(ProxyError::NoAvailableChannel(
            "所有优先级的渠道都不可用".to_string(),
        ))
    }

    /// 获取渠道信息（带缓存）
    async fn get_channel(&self, channel_id: &str) -> Result<ChannelInfo, ProxyError> {
        // 1. 检查缓存
        if let Some(channel) = self.cache.get_channel(channel_id).await {
            return Ok(channel);
        }

        // 2. 缓存未命中，查询数据库
        let result = sqlx::query_as::<_, (String, String, String, String, String, String, i32, i32)>(
            "SELECT id, name, api_keys, endpoints, models, custom_headers, COALESCE(timeout_secs, 300), COALESCE(max_concurrency, 0) FROM channels WHERE id = ? AND enabled = 1"
        )
        .bind(channel_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        let (id, name, api_keys_str, endpoints_str, models_str, custom_headers_str, timeout_secs, max_concurrency) =
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
            timeout_secs: timeout_secs as u64,
            max_concurrency: max_concurrency as u32,
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
        let channels = sqlx::query_as::<_, (String, String, String, String, String, String, i32, i32)>(
            "SELECT id, name, api_keys, endpoints, models, custom_headers, COALESCE(timeout_secs, 300), COALESCE(max_concurrency, 0) FROM channels WHERE enabled = 1",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        for (id, name, api_keys_str, endpoints_str, models_str, custom_headers_str, timeout_secs, max_concurrency) in channels {
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
                timeout_secs: timeout_secs as u64,
                max_concurrency: max_concurrency as u32,
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
        let selection =
            match select_channel_for_proxy(state, headers, body, client_endpoint, &exclude_ids)
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    // 渠道选择失败时也记录日志
                    save_request_record(
                        state,
                        Some(request_id.to_string()),
                        api_key_id,
                        None,
                        &model,
                        request_content.clone(),
                        None,
                        &attempts,
                        None,
                        true,
                        user_agent.clone(),
                        Some(&e),
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
                request_id.to_string(),
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
                        Some(request_id.to_string()),
                        api_key_id,
                        group_id.as_deref(),
                        &model,
                        request_content.clone(),
                        None,
                        &attempts,
                        None,
                        true,
                        user_agent.clone(),
                        Some(&e),
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
        Some(request_id.to_string()),
        api_key_id,
        None,
        &model,
        request_content,
        None,
        &attempts,
        None,
        true,
        user_agent,
        None,
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

    let request_id = crate::api::response::generate_id();
    let model = body["model"].as_str().unwrap_or("unknown");
    let is_stream = body["stream"].as_bool().unwrap_or(false);
    let api_key_id = Some(auth.key_id.as_str());

    // 速率限制检查（RPM + TPM）
    if let Err(retry_after) = state.rate_limiter.check_rpm(&auth.key_id, auth.rate_limit_rpm).await {
        return format_rate_limit_error(retry_after, "RPM", auth.rate_limit_rpm, error_format);
    }
    if let Err(retry_after) = state.rate_limiter.check_tpm(&auth.key_id, auth.rate_limit_tpm).await {
        return format_rate_limit_error(retry_after, "TPM", auth.rate_limit_tpm, error_format);
    }

    // 预算检查（月/日额度）
    if let Err(msg) = check_budget(&state.pool, &auth.key_id).await {
        return format_budget_error(&msg, error_format);
    }

    // 验证 API Key 是否有权访问目标模型（三段式：403→404→503）
    if let Err(e) = validate_model_access(&state.pool, &auth.key_id, model, &auth.allowed_groups).await {
        return format_proxy_error(e, error_format);
    }

    if is_stream {
        match proxy_stream(state, &request_id, api_key_id, &headers, &body, client_endpoint).await {
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
        match proxy_request(state, &request_id, api_key_id, &headers, &body, client_endpoint).await {
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
        let allowed: Vec<&str> = allowed_groups.split(',')
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
            let allowed = crate::api::handlers::admin::api_keys::parse_supported_models(&models_str);
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
    // allowed_groups 为空时跳过，由后续 select_group_item 检查渠道可用性
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
            return Err(ProxyError::ModelNotFound(format!(
                "模型不存在: {}",
                model
            )));
        }
    }

    // === 第三关：分组内是否有可用渠道（延迟到 select_group_item 时检查）===
    Ok(())
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

/// 格式化 402 预算超限错误
fn format_budget_error(
    msg: &str,
    error_format: &ErrorFormat,
) -> axum::response::Response {
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
