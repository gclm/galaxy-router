//! 调度器状态（Step D 拆分：ChannelStatus 独立到 channel_status.rs；
//! LoadBalancerState 含私有字段，impl 不可跨文件，留此）。

mod channel_status;

pub use channel_status::ChannelStatus;

use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::circuit::{CircuitBreaker, CircuitConfig};
use super::sticky::StickySessionManager;
use crate::scheduler::capacity::ChannelCapacityManager;
use crate::scheduler::metrics::SchedulerMetrics;
use crate::scheduler::runtime::{ChannelRuntimeManager, ChannelRuntimeStats};

/// 负载均衡状态
#[derive(Clone)]
pub struct LoadBalancerState {
    /// 渠道状态
    pub channel_states: Arc<RwLock<HashMap<String, ChannelStatus>>>,
    /// 粘性会话管理器
    sticky_manager: StickySessionManager,
    /// 熔断器（新增）
    pub circuit_breaker: CircuitBreaker,
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

    /// 记录请求失败（渠道级黑名单使用渠道自身的阈值与时长）
    pub async fn record_failure(&self, channel_id: &str, should_blacklist: bool) {
        // 更新统计
        {
            let mut states = self.channel_states.write().await;
            if let Some(status) = states.get_mut(channel_id) {
                status.record_failure();

                // 检查是否需要拉黑
                if should_blacklist && status.failure_count >= status.blacklist_threshold {
                    status.blacklist(status.blacklist_minutes);
                    tracing::warn!(
                        "渠道 {} 被拉黑 {} 分钟 (failure_count={})",
                        channel_id,
                        status.blacklist_minutes,
                        status.failure_count
                    );
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
    ///
    /// `failure_threshold` / `blacklist_minutes` 来自 channels 表，用于渠道级黑名单判定。
    /// 每次调用都会同步最新配置到 ChannelStatus，以便用户在 DB 修改后能立即生效。
    pub async fn ensure_channel_status(
        &self,
        channel_id: &str,
        max_concurrency: u32,
        failure_threshold: u64,
        blacklist_minutes: i64,
    ) {
        let states = self.channel_states.read().await;
        if states.contains_key(channel_id) {
            drop(states);
            let mut states = self.channel_states.write().await;
            if let Some(status) = states.get_mut(channel_id) {
                status.max_concurrency = max_concurrency;
                status.blacklist_threshold = failure_threshold;
                status.blacklist_minutes = blacklist_minutes;
            }
            return;
        }
        drop(states);
        let mut states = self.channel_states.write().await;
        // Double-check after acquiring write lock
        if let Some(status) = states.get_mut(channel_id) {
            status.max_concurrency = max_concurrency;
            status.blacklist_threshold = failure_threshold;
            status.blacklist_minutes = blacklist_minutes;
            return;
        }
        let mut status = ChannelStatus::new(channel_id);
        status.max_concurrency = max_concurrency;
        status.blacklist_threshold = failure_threshold;
        status.blacklist_minutes = blacklist_minutes;
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
        let threshold = 3u64;
        lb.ensure_channel_status("ch1", 10, threshold, 10).await;

        // 初始可用
        assert!(lb.is_channel_available("ch1").await);

        // 连续失败达阈值 → channel 拉黑
        for _ in 0..threshold {
            lb.record_failure("ch1", true).await;
        }
        assert!(
            !lb.is_channel_available("ch1").await,
            "达阈值的 channel 应被拉黑"
        );
    }
}
