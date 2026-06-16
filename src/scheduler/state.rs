use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tokio::sync::RwLock;

use super::circuit::{CircuitBreaker, CircuitConfig};
use super::sticky::StickySessionManager;
use crate::scheduler::capacity::ChannelCapacityManager;
use crate::scheduler::metrics::SchedulerMetrics;
use crate::scheduler::runtime::{ChannelRuntimeManager, ChannelRuntimeStats};

/// 渠道状态
#[derive(Debug)]
pub struct ChannelStatus {
    pub channel_id: String,
    pub success_count: u64,
    pub failure_count: u64,
    pub last_success: Option<DateTime<Utc>>,
    pub last_failure: Option<DateTime<Utc>>,
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
            avg_latency_ms: 0.0,
            is_blacklisted: false,
            blacklist_until: None,
            active_requests: Arc::new(AtomicU64::new(0)),
            last_used_at: Arc::new(RwLock::new(Instant::now())),
            max_concurrency: 0,
        }
    }

    /// 计算错误率
    #[cfg(test)]
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

/// 负载均衡状态
#[derive(Clone)]
pub struct LoadBalancerState {
    /// 渠道状态
    pub channel_states: Arc<RwLock<HashMap<String, ChannelStatus>>>,
    /// 粘性会话管理器
    sticky_manager: StickySessionManager,
    /// 熔断器（新增）
    pub circuit_breaker: CircuitBreaker,
    /// 拉黑阈值（连续失败次数）
    pub blacklist_threshold: u64,
    /// 拉黑时长（分钟）
    pub blacklist_minutes: i64,
    /// 容量管理器（RAII permit 方式跟踪并发）
    capacity_manager: ChannelCapacityManager,
    /// 调度器运行时指标
    scheduler_metrics: SchedulerMetrics,
    /// 渠道运行时 EWMA 统计
    runtime_manager: ChannelRuntimeManager,
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
            sticky_manager: StickySessionManager::default(),
            circuit_breaker: CircuitBreaker::new(CircuitConfig::default()),
            blacklist_threshold: 3,
            blacklist_minutes: 10,
            capacity_manager: ChannelCapacityManager::new(),
            scheduler_metrics: SchedulerMetrics::new(),
            runtime_manager: ChannelRuntimeManager::new(),
        }
    }

    /// 获取共享的容量管理器
    pub fn capacity_manager(&self) -> ChannelCapacityManager {
        self.capacity_manager.clone()
    }
    /// 记录真实调度选择结果
    pub fn record_scheduler_selection(
        &self,
        selected_channel_id: &str,
        selected_was_sticky: bool,
        sticky_channel_id: Option<&str>,
    ) {
        if selected_was_sticky {
            self.scheduler_metrics.record_sticky_hit();
            return;
        }

        self.scheduler_metrics.record_load_balance();
        if let Some(sticky_channel_id) = sticky_channel_id
            && sticky_channel_id != selected_channel_id
        {
            self.scheduler_metrics.record_channel_switch();
        }
    }

    /// 获取渠道运行时统计快照
    pub fn runtime_stats(&self, channel_id: &str) -> ChannelRuntimeStats {
        self.runtime_manager.get_stats(channel_id)
    }

    /// 记录请求成功
    pub async fn record_success(&self, channel_id: &str, latency_ms: f64) {
        self.record_success_with_ttft(channel_id, latency_ms, None)
            .await;
    }

    /// 记录请求成功（流式路径可传入 TTFT）
    pub async fn record_success_with_ttft(
        &self,
        channel_id: &str,
        latency_ms: f64,
        ttft_ms: Option<f64>,
    ) {
        // 更新统计
        {
            let mut states = self.channel_states.write().await;
            if let Some(status) = states.get_mut(channel_id) {
                status.record_success(latency_ms).await;
            }
        }
        self.runtime_manager
            .record_success(channel_id, latency_ms, ttft_ms);
        // per-key 熔断由 executor 在 key 循环内直接调用 circuit_breaker
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
        self.runtime_manager.record_failure(channel_id);
        // per-key 熔断由 executor 在 key 循环内直接调用 circuit_breaker
    }

    /// 检查渠道是否可用（channel 级：查黑名单）
    ///
    /// per-key 熔断由 executor 在 key 循环内通过 `circuit_breaker` 直接检查；
    /// 此处只做 channel 级粗过滤（全 key 失败导致的黑名单）。
    pub async fn is_channel_available(&self, channel_id: &str) -> bool {
        let states = self.channel_states.read().await;
        match states.get(channel_id) {
            None => true, // 无运行时统计，视为可用
            Some(status) => status.is_available(),
        }
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

    /// 获取粘性会话
    pub async fn get_sticky_session(&self, session_hash: &str) -> Option<String> {
        self.sticky_manager.get(session_hash).await
    }

    /// 设置粘性会话
    pub async fn set_sticky_session(&self, session_hash: &str, channel_id: &str) {
        self.sticky_manager.set(session_hash, channel_id).await
    }

    /// 记录 monitor 探测成功
    pub fn record_monitor_success(&self, channel_id: &str) {
        self.runtime_manager.record_monitor_success(channel_id);
    }

    /// 记录 monitor 探测失败
    pub fn record_monitor_failure(&self, channel_id: &str) {
        self.runtime_manager.record_monitor_failure(channel_id);
    }

    /// 清理过期的粘性会话
    pub async fn cleanup_expired_sessions(&self) {
        self.sticky_manager.cleanup_expired().await
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

    #[test]
    fn scheduler_metrics_records_sticky_and_load_balance_selections() {
        let lb = LoadBalancerState::new();

        lb.record_scheduler_selection("ch-sticky", true, Some("ch-sticky"));
        lb.record_scheduler_selection("ch-a", false, None);

        let snap = lb.scheduler_metrics.snapshot();
        assert_eq!(snap.sticky_hits, 1);
        assert_eq!(snap.load_balance_selects, 1);
        assert_eq!(snap.channel_switches, 0);
        assert_eq!(snap.total_selections, 2);
    }

    #[test]
    fn scheduler_metrics_records_channel_switch_from_sticky() {
        let lb = LoadBalancerState::new();

        lb.record_scheduler_selection("ch-new", false, Some("ch-old"));

        let snap = lb.scheduler_metrics.snapshot();
        assert_eq!(snap.sticky_hits, 0);
        assert_eq!(snap.load_balance_selects, 1);
        assert_eq!(snap.channel_switches, 1);
        assert_eq!(snap.total_selections, 1);
    }

    #[tokio::test]
    async fn runtime_manager_updates_on_production_success_and_failure() {
        let lb = LoadBalancerState::new();

        lb.record_failure("ch-runtime", false).await;
        let after_failure = lb.runtime_stats("ch-runtime").error_rate();
        assert!(after_failure > 0.0);

        lb.record_success("ch-runtime", 120.0).await;
        let stats = lb.runtime_stats("ch-runtime");
        assert!(stats.error_rate() < after_failure);
        assert_eq!(stats.avg_latency_ms(), 120.0);
        assert_eq!(stats.request_count, 2);
    }

    #[tokio::test]
    async fn runtime_manager_tracks_ttft_and_monitor_health() {
        let lb = LoadBalancerState::new();

        lb.record_success_with_ttft("ch-stream", 300.0, Some(42.0))
            .await;
        lb.record_monitor_failure("ch-stream");

        let stats = lb.runtime_stats("ch-stream");
        assert_eq!(stats.avg_latency_ms(), 300.0);
        assert_eq!(stats.avg_ttft_ms(), 42.0);
        assert!(stats.health() < 1.0);
    }

    #[tokio::test]
    async fn is_channel_available_reflects_blacklist() {
        // is_channel_available 走 channel 级黑名单（全 key 失败才拉黑），
        // 不再查 circuit_breaker[default]
        let lb = LoadBalancerState::new();
        lb.ensure_channel_status("ch1", 10).await;

        // 初始可用
        assert!(lb.is_channel_available("ch1").await);

        // 连续失败达阈值 → channel 拉黑
        for _ in 0..lb.blacklist_threshold {
            lb.record_failure("ch1", true).await;
        }
        assert!(
            !lb.is_channel_available("ch1").await,
            "达阈值的 channel 应被拉黑"
        );
    }
}
