use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// EWMA 平滑因子：error rate
const ERROR_ALPHA: f64 = 0.3;
/// EWMA 平滑因子：latency / TTFT
const LATENCY_ALPHA: f64 = 0.2;
/// monitor 探测成功恢复量
const HEALTH_RECOVERY: f64 = 0.2;
/// monitor 探测失败惩罚量
const HEALTH_PENALTY: f64 = 0.3;

/// 单渠道运行时统计（EWMA）
#[derive(Debug, Clone)]
pub struct ChannelRuntimeStats {
    error_ewma: f64,
    latency_ewma: f64,
    ttft_ewma: f64,
    health: f64,
    request_count: u64,
}

impl Default for ChannelRuntimeStats {
    fn default() -> Self {
        Self {
            error_ewma: 0.0,
            latency_ewma: 0.0,
            ttft_ewma: 0.0,
            health: 1.0,
            request_count: 0,
        }
    }
}

impl ChannelRuntimeStats {
    pub fn new() -> Self {
        Self::default()
    }

    /// 记录请求成功，更新 latency 和可选 TTFT
    pub fn record_success(&mut self, latency_ms: f64, ttft_ms: Option<f64>) {
        // EWMA: error 衰减（成功 = 0）
        self.error_ewma *= 1.0 - ERROR_ALPHA;
        // EWMA: latency 更新
        self.update_latency_ewma(latency_ms);
        // EWMA: TTFT 更新（仅流式请求）
        if let Some(ttft) = ttft_ms {
            self.update_ttft_ewma(ttft);
        }
        self.request_count += 1;
    }

    /// 记录请求失败，更新 error rate
    pub fn record_failure(&mut self) {
        // EWMA: error 上升（失败 = 1）
        self.error_ewma = ERROR_ALPHA + (1.0 - ERROR_ALPHA) * self.error_ewma;
        self.request_count += 1;
    }

    /// 记录 monitor 探测成功
    pub fn record_monitor_success(&mut self) {
        self.health = (self.health + HEALTH_RECOVERY).min(1.0);
    }

    /// 记录 monitor 探测失败
    pub fn record_monitor_failure(&mut self) {
        self.health = (self.health - HEALTH_PENALTY).max(0.0);
    }

    pub fn error_rate(&self) -> f64 {
        self.error_ewma
    }

    pub fn avg_latency_ms(&self) -> f64 {
        self.latency_ewma
    }

    pub fn avg_ttft_ms(&self) -> f64 {
        self.ttft_ewma
    }

    pub fn health(&self) -> f64 {
        self.health
    }

    pub fn request_count(&self) -> u64 {
        self.request_count
    }

    fn update_latency_ewma(&mut self, latency_ms: f64) {
        if self.latency_ewma == 0.0 {
            self.latency_ewma = latency_ms;
        } else {
            self.latency_ewma =
                LATENCY_ALPHA * latency_ms + (1.0 - LATENCY_ALPHA) * self.latency_ewma;
        }
    }

    fn update_ttft_ewma(&mut self, ttft_ms: f64) {
        if self.ttft_ewma == 0.0 {
            self.ttft_ewma = ttft_ms;
        } else {
            self.ttft_ewma = LATENCY_ALPHA * ttft_ms + (1.0 - LATENCY_ALPHA) * self.ttft_ewma;
        }
    }
}

/// 多渠道运行时统计管理器
#[derive(Debug, Clone, Default)]
pub struct ChannelRuntimeManager {
    stats: Arc<Mutex<HashMap<String, ChannelRuntimeStats>>>,
}

impl ChannelRuntimeManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_success(&self, channel_id: &str, latency_ms: f64, ttft_ms: Option<f64>) {
        let mut stats = self.stats.lock().expect("runtime stats mutex poisoned");
        stats
            .entry(channel_id.to_string())
            .or_default()
            .record_success(latency_ms, ttft_ms);
    }

    pub fn record_failure(&self, channel_id: &str) {
        let mut stats = self.stats.lock().expect("runtime stats mutex poisoned");
        stats
            .entry(channel_id.to_string())
            .or_default()
            .record_failure();
    }

    pub fn record_monitor_success(&self, channel_id: &str) {
        let mut stats = self.stats.lock().expect("runtime stats mutex poisoned");
        stats
            .entry(channel_id.to_string())
            .or_default()
            .record_monitor_success();
    }

    pub fn record_monitor_failure(&self, channel_id: &str) {
        let mut stats = self.stats.lock().expect("runtime stats mutex poisoned");
        stats
            .entry(channel_id.to_string())
            .or_default()
            .record_monitor_failure();
    }

    pub fn get_stats(&self, channel_id: &str) -> ChannelRuntimeStats {
        let stats = self.stats.lock().expect("runtime stats mutex poisoned");
        stats.get(channel_id).cloned().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P6.1: 失败后成功，error_rate 下降但不归零
    #[test]
    fn runtime_error_ewma_decreases_after_success_but_not_zero() {
        let mut stats = ChannelRuntimeStats::new();

        // 初始 error_rate = 0
        assert_eq!(stats.error_rate(), 0.0);

        // 记录失败
        stats.record_failure();
        let after_failure = stats.error_rate();
        assert!(
            after_failure > 0.0,
            "error rate should be positive after failure, got {}",
            after_failure
        );

        // 记录成功
        stats.record_success(100.0, None);
        let after_success = stats.error_rate();
        assert!(
            after_success < after_failure,
            "error rate should decrease after success: {} vs {}",
            after_success,
            after_failure
        );
        assert!(
            after_success > 0.0,
            "error rate should NOT be zero after one success, got {}",
            after_success
        );
    }

    /// P6.1: 连续失败后 error_rate 接近 1.0
    #[test]
    fn runtime_error_ewma_converges_on_repeated_failures() {
        let mut stats = ChannelRuntimeStats::new();
        for _ in 0..20 {
            stats.record_failure();
        }
        assert!(
            stats.error_rate() > 0.99,
            "error rate should converge near 1.0 after many failures, got {}",
            stats.error_rate()
        );
    }

    /// P6.1: 连续成功后 error_rate 接近 0.0
    #[test]
    fn runtime_error_ewma_converges_on_repeated_successes() {
        let mut stats = ChannelRuntimeStats::new();
        // 先建立一些失败
        for _ in 0..5 {
            stats.record_failure();
        }
        // 大量成功后应接近 0
        for _ in 0..50 {
            stats.record_success(100.0, None);
        }
        assert!(
            stats.error_rate() < 0.01,
            "error rate should converge near 0.0 after many successes, got {}",
            stats.error_rate()
        );
    }

    /// P6.2: TTFT EWMA 跟踪流式首 token 延迟
    #[test]
    fn runtime_ttft_ewma_tracks_first_token_latency() {
        let mut stats = ChannelRuntimeStats::new();
        assert_eq!(stats.avg_ttft_ms(), 0.0);

        stats.record_success(200.0, Some(50.0));
        assert!(
            stats.avg_ttft_ms() > 0.0,
            "TTFT should be tracked after streaming success"
        );

        // TTFT 应与总延迟不同
        assert!(
            stats.avg_ttft_ms() < stats.avg_latency_ms(),
            "TTFT ({}) should be less than total latency ({})",
            stats.avg_ttft_ms(),
            stats.avg_latency_ms()
        );
    }

    /// P6.2: 非流式请求不影响 TTFT
    #[test]
    fn runtime_ttft_unchanged_without_streaming() {
        let mut stats = ChannelRuntimeStats::new();
        stats.record_success(100.0, None);
        assert_eq!(
            stats.avg_ttft_ms(),
            0.0,
            "TTFT should stay 0 without streaming"
        );
    }

    /// P6.4: monitor 探测失败降低 health factor
    #[test]
    fn runtime_monitor_failure_lowers_health() {
        let mut stats = ChannelRuntimeStats::new();
        assert_eq!(stats.health(), 1.0);

        stats.record_monitor_failure();
        assert!(
            stats.health() < 1.0,
            "health should decrease after monitor failure, got {}",
            stats.health()
        );

        // 连续失败最终降到 0
        for _ in 0..10 {
            stats.record_monitor_failure();
        }
        assert_eq!(
            stats.health(),
            0.0,
            "health should reach 0 after enough failures"
        );
    }

    /// P6.4: monitor 探测成功恢复 health，但上限为 1.0
    #[test]
    fn runtime_monitor_success_recovers_health_capped_at_one() {
        let mut stats = ChannelRuntimeStats::new();
        stats.record_monitor_failure();
        let after_failure = stats.health();
        assert!(after_failure < 1.0);

        stats.record_monitor_success();
        assert!(
            stats.health() > after_failure,
            "health should increase after monitor success"
        );

        // 连续成功不超过 1.0
        for _ in 0..20 {
            stats.record_monitor_success();
        }
        assert!(
            stats.health() <= 1.0,
            "health should not exceed 1.0, got {}",
            stats.health()
        );
    }

    /// Manager 集成：多渠道独立跟踪
    #[test]
    fn runtime_manager_tracks_channels_independently() {
        let mgr = ChannelRuntimeManager::new();
        mgr.record_failure("ch-a");
        mgr.record_success("ch-b", 100.0, None);

        let a = mgr.get_stats("ch-a");
        let b = mgr.get_stats("ch-b");

        assert!(a.error_rate() > b.error_rate());
        assert!(b.avg_latency_ms() > 0.0);
        assert_eq!(a.request_count(), 1);
        assert_eq!(b.request_count(), 1);
    }
}
