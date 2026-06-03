use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tokio::sync::RwLock;

use super::circuit::{CircuitBreaker, CircuitConfig};

/// 渠道状态
#[derive(Debug)]
pub struct ChannelStatus {
    pub channel_id: String,
    pub success_count: u64,
    pub failure_count: u64,
    pub last_success: Option<DateTime<Utc>>,
    pub last_failure: Option<DateTime<Utc>>,
    pub last_health_check: Option<DateTime<Utc>>,
    pub avg_latency_ms: f64,
    pub is_blacklisted: bool,
    pub blacklist_until: Option<DateTime<Utc>>,
    /// 当前活跃请求数
    pub active_requests: Arc<AtomicU64>,
    /// 最后一次使用时间（用于 LRU 选择）
    pub last_used_at: Arc<RwLock<Instant>>,
    /// 最大并发数（0=不限制）
    pub max_concurrency: u32,
}

impl ChannelStatus {
    /// 创建新的渠道状态
    pub fn new(channel_id: &str) -> Self {
        Self {
            channel_id: channel_id.to_string(),
            success_count: 0,
            failure_count: 0,
            last_success: None,
            last_failure: None,
            last_health_check: None,
            avg_latency_ms: 0.0,
            is_blacklisted: false,
            blacklist_until: None,
            active_requests: Arc::new(AtomicU64::new(0)),
            last_used_at: Arc::new(RwLock::new(Instant::now())),
            max_concurrency: 0,
        }
    }

    /// 计算错误率
    pub fn error_rate(&self) -> f64 {
        let total = self.success_count + self.failure_count;
        if total == 0 {
            return 0.0;
        }
        self.failure_count as f64 / total as f64
    }

    /// 计算负载率 (0-100+)
    pub fn load_rate(&self) -> u32 {
        if self.max_concurrency == 0 {
            return 0;
        }
        let active = self.active_requests.load(Ordering::Relaxed);
        (active * 100 / self.max_concurrency as u64) as u32
    }

    /// 活跃请求 +1
    pub fn increment_active(&self) {
        self.active_requests.fetch_add(1, Ordering::Relaxed);
    }

    /// 活跃请求 -1
    pub fn decrement_active(&self) {
        self.active_requests.fetch_sub(1, Ordering::Relaxed);
    }

    /// 记录成功
    pub async fn record_success(&mut self, latency_ms: f64) {
        self.success_count += 1;
        self.last_success = Some(Utc::now());

        // 成功时将失败计数减半（衰减而非清零），避免单次成功洗白
        self.failure_count = self.failure_count.saturating_div(2);

        // 解除拉黑
        if self.failure_count == 0 {
            self.is_blacklisted = false;
            self.blacklist_until = None;
        }

        // 更新平均延迟（指数移动平均）
        if self.avg_latency_ms == 0.0 {
            self.avg_latency_ms = latency_ms;
        } else {
            self.avg_latency_ms = 0.8 * self.avg_latency_ms + 0.2 * latency_ms;
        }

        // 更新最后使用时间
        *self.last_used_at.write().await = Instant::now();
    }

    /// 记录健康探测结果
    pub fn record_health_check(&mut self, success: bool) {
        self.last_health_check = Some(Utc::now());
        if !success {
            self.record_failure();
        }
    }

    /// 记录失败
    pub fn record_failure(&mut self) {
        self.failure_count += 1;
        self.last_failure = Some(Utc::now());
    }

    /// 拉黑
    pub fn blacklist(&mut self, minutes: i64) {
        self.is_blacklisted = true;
        self.blacklist_until = Some(Utc::now() + chrono::Duration::minutes(minutes));
    }

    /// 检查是否可用
    pub fn is_available(&self) -> bool {
        if !self.is_blacklisted {
            return true;
        }

        // 检查拉黑是否过期
        if let Some(until) = self.blacklist_until {
            Utc::now() >= until
        } else {
            false
        }
    }
}

/// 粘性会话
#[derive(Debug, Clone)]
pub struct StickySession {
    pub channel_id: String,
    pub expires_at: DateTime<Utc>,
}

/// 渠道负载信息（用于选择算法）
#[derive(Clone)]
pub struct ChannelLoadInfo {
    pub active_requests: u64,
    pub load_rate: u32,
    pub last_used_at: Instant,
    pub max_concurrency: u32,
}

/// 负载均衡状态
#[derive(Clone)]
pub struct LoadBalancerState {
    /// 渠道状态
    pub channel_states: Arc<RwLock<HashMap<String, ChannelStatus>>>,
    /// 粘性会话
    pub sticky_sessions: Arc<RwLock<HashMap<String, StickySession>>>,
    /// 熔断器（新增）
    pub circuit_breaker: CircuitBreaker,
    /// 粘性会话 TTL（秒）
    pub sticky_ttl_secs: i64,
    /// 拉黑阈值（连续失败次数）
    pub blacklist_threshold: u64,
    /// 拉黑时长（分钟）
    pub blacklist_minutes: i64,
    /// 粘性会话最大容量
    max_sticky_sessions: usize,
}

impl Default for LoadBalancerState {
    fn default() -> Self {
        Self::new()
    }
}

impl LoadBalancerState {
    pub fn new() -> Self {
        Self {
            channel_states: Arc::new(RwLock::new(HashMap::new())),
            sticky_sessions: Arc::new(RwLock::new(HashMap::new())),
            circuit_breaker: CircuitBreaker::new(CircuitConfig::default()),
            sticky_ttl_secs: 3600,
            blacklist_threshold: 3,
            blacklist_minutes: 10,
            max_sticky_sessions: 10000,
        }
    }

    /// 记录请求成功
    pub async fn record_success(&self, channel_id: &str, latency_ms: f64) {
        // 更新统计
        {
            let mut states = self.channel_states.write().await;
            if let Some(status) = states.get_mut(channel_id) {
                status.record_success(latency_ms).await;
            }
        }
        // 通知熔断器（使用空的 key_hint，后续可扩展）
        self.circuit_breaker
            .record_success(channel_id, "default")
            .await;
    }

    /// 记录请求失败
    pub async fn record_failure(&self, channel_id: &str, should_blacklist: bool) {
        // 更新统计
        {
            let mut states = self.channel_states.write().await;
            if let Some(status) = states.get_mut(channel_id) {
                status.record_failure();

                // 检查是否需要拉黑
                if should_blacklist && status.failure_count >= self.blacklist_threshold {
                    status.blacklist(self.blacklist_minutes);
                    tracing::warn!("渠道 {} 被拉黑 {} 分钟", channel_id, self.blacklist_minutes);
                }
            }
        }
        // 通知熔断器
        self.circuit_breaker
            .record_failure(channel_id, "default")
            .await;
    }

    /// 检查渠道是否可用（使用熔断器）
    pub async fn is_channel_available(&self, channel_id: &str) -> bool {
        let (tripped, _) = self.circuit_breaker.is_tripped(channel_id, "default").await;
        !tripped
    }

    /// 确保渠道状态存在（惰性初始化）
    pub async fn ensure_channel_status(&self, channel_id: &str, max_concurrency: u32) {
        let states = self.channel_states.read().await;
        if states.contains_key(channel_id) {
            // 更新 max_concurrency 如果已存在
            drop(states);
            let mut states = self.channel_states.write().await;
            if let Some(status) = states.get_mut(channel_id) {
                status.max_concurrency = max_concurrency;
            }
            return;
        }
        drop(states);
        let mut states = self.channel_states.write().await;
        // Double-check after acquiring write lock
        if let Some(status) = states.get_mut(channel_id) {
            status.max_concurrency = max_concurrency;
            return;
        }
        let mut status = ChannelStatus::new(channel_id);
        status.max_concurrency = max_concurrency;
        states.insert(channel_id.to_string(), status);
    }

    /// 活跃请求 +1
    pub async fn increment_active(&self, channel_id: &str) {
        let states = self.channel_states.read().await;
        if let Some(status) = states.get(channel_id) {
            status.increment_active();
        }
    }

    /// 活跃请求 -1
    pub async fn decrement_active(&self, channel_id: &str) {
        let states = self.channel_states.read().await;
        if let Some(status) = states.get(channel_id) {
            status.decrement_active();
        }
    }

    /// 获取渠道的负载信息（负载率、活跃请求数、最后使用时间）
    pub async fn get_channel_load_info(&self, channel_id: &str) -> Option<ChannelLoadInfo> {
        let states = self.channel_states.read().await;
        let status = states.get(channel_id)?;
        let active = status.active_requests.load(Ordering::Relaxed);
        let load_rate = status.load_rate();
        let last_used = *status.last_used_at.read().await;
        Some(ChannelLoadInfo {
            active_requests: active,
            load_rate,
            last_used_at: last_used,
            max_concurrency: status.max_concurrency,
        })
    }

    /// 计算渠道评分
    pub async fn calculate_score(&self, channel_id: &str, base_weight: i32) -> f64 {
        let states = self.channel_states.read().await;
        let status = states.get(channel_id);

        let mut score = base_weight as f64;

        if let Some(status) = status {
            // 不可用渠道评分归零
            if !status.is_available() {
                return 0.0;
            }

            // 错误率惩罚
            let error_rate = status.error_rate();
            score *= 1.0 - error_rate;

            // 延迟惩罚（延迟越高，评分越低）
            if status.avg_latency_ms > 0.0 {
                let latency_factor = 1.0 / (1.0 + status.avg_latency_ms / 1000.0);
                score *= latency_factor;
            }
        }

        score.max(0.0)
    }

    /// 获取粘性会话
    pub async fn get_sticky_session(&self, session_hash: &str) -> Option<String> {
        let sessions = self.sticky_sessions.read().await;
        if let Some(session) = sessions.get(session_hash)
            && Utc::now() < session.expires_at
        {
            return Some(session.channel_id.clone());
        }
        None
    }

    /// 设置粘性会话
    pub async fn set_sticky_session(&self, session_hash: &str, channel_id: &str) {
        let mut sessions = self.sticky_sessions.write().await;

        // 容量检查：超过上限时清理过期条目
        if sessions.len() >= self.max_sticky_sessions {
            let now = Utc::now();
            sessions.retain(|_, session| now < session.expires_at);

            // 清理后仍然满，拒绝新 session
            if sessions.len() >= self.max_sticky_sessions {
                tracing::warn!(
                    "粘性会话已满（{}），拒绝新 session: {}",
                    self.max_sticky_sessions,
                    session_hash
                );
                return;
            }
        }

        let now = Utc::now();
        sessions.insert(
            session_hash.to_string(),
            StickySession {
                channel_id: channel_id.to_string(),
                expires_at: now + chrono::Duration::seconds(self.sticky_ttl_secs),
            },
        );
    }

    /// 清理过期的粘性会话
    pub async fn cleanup_expired_sessions(&self) {
        let mut sessions = self.sticky_sessions.write().await;
        let now = Utc::now();
        sessions.retain(|_, session| now < session.expires_at);
    }

    /// 清理过期的拉黑
    pub async fn cleanup_expired_blacklists(&self) {
        let mut states = self.channel_states.write().await;
        for (_, status) in states.iter_mut() {
            if status.is_blacklisted
                && let Some(until) = status.blacklist_until
                && Utc::now() >= until
            {
                status.is_blacklisted = false;
                status.blacklist_until = None;
                status.failure_count = 0; // 重置失败计数
                tracing::info!("渠道 {} 拉黑已过期，恢复正常", status.channel_id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_status() -> ChannelStatus {
        ChannelStatus::new("ch-1")
    }

    #[test]
    fn error_rate_with_no_traffic_is_zero() {
        let s = sample_status();
        assert_eq!(s.error_rate(), 0.0);
    }

    #[test]
    fn error_rate_reflects_failure_ratio() {
        let mut s = sample_status();
        s.success_count = 3;
        s.failure_count = 1;
        assert!((s.error_rate() - 0.25).abs() < 1e-9);
    }

    #[tokio::test]
    async fn record_success_halves_failure_count() {
        let mut s = sample_status();
        s.failure_count = 4;
        s.record_success(100.0).await;
        assert_eq!(s.failure_count, 2);
        s.record_success(100.0).await;
        assert_eq!(s.failure_count, 1);
        s.record_success(100.0).await;
        assert_eq!(s.failure_count, 0);
    }

    #[tokio::test]
    async fn blacklist_unblocks_when_failures_decay_to_zero() {
        let mut s = sample_status();
        s.failure_count = 2;
        s.blacklist(10);
        assert!(s.is_blacklisted);
        // 多次成功将失败计数衰减到 0 → 自动解封
        for _ in 0..5 {
            s.record_success(50.0).await;
        }
        assert!(!s.is_blacklisted);
        assert!(s.blacklist_until.is_none());
    }

    #[test]
    fn is_available_expires_blacklist_by_time() {
        let mut s = sample_status();
        s.blacklist(-1); // 立即过期
        assert!(s.is_available());
    }

    #[test]
    fn load_rate_is_zero_when_no_max_concurrency() {
        let s = sample_status();
        assert_eq!(s.load_rate(), 0);
    }

    #[test]
    fn load_rate_computes_percentage() {
        let s = sample_status();
        s.active_requests.store(5, Ordering::Relaxed);
        // Need to set max_concurrency
        let mut s = s;
        s.max_concurrency = 10;
        assert_eq!(s.load_rate(), 50);
    }

    #[test]
    fn active_increment_decrement() {
        let s = sample_status();
        assert_eq!(s.active_requests.load(Ordering::Relaxed), 0);
        s.increment_active();
        s.increment_active();
        assert_eq!(s.active_requests.load(Ordering::Relaxed), 2);
        s.decrement_active();
        assert_eq!(s.active_requests.load(Ordering::Relaxed), 1);
    }
}
