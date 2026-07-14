use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// 调度器运行时指标（原子计数器，无锁读写）
#[derive(Debug, Clone)]
pub struct SchedulerMetrics {
    inner: Arc<SchedulerMetricsInner>,
}

#[derive(Debug, Default)]
struct SchedulerMetricsInner {
    sticky_hits: AtomicU64,
    load_balance_selects: AtomicU64,
    channel_switches: AtomicU64,
    total_selections: AtomicU64,
}

impl SchedulerMetrics {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(SchedulerMetricsInner::default()),
        }
    }

    /// 记录粘性会话命中
    pub fn record_sticky_hit(&self) {
        self.inner.sticky_hits.fetch_add(1, Ordering::Relaxed);
        self.inner.total_selections.fetch_add(1, Ordering::Relaxed);
    }

    /// 记录负载均衡选择
    pub fn record_load_balance(&self) {
        self.inner
            .load_balance_selects
            .fetch_add(1, Ordering::Relaxed);
        self.inner.total_selections.fetch_add(1, Ordering::Relaxed);
    }

    /// 记录渠道切换（请求使用了与 sticky 不同的渠道）
    pub fn record_channel_switch(&self) {
        self.inner.channel_switches.fetch_add(1, Ordering::Relaxed);
    }

}

impl Default for SchedulerMetrics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SchedulerMetricsSnapshot {
    pub sticky_hits: u64,
    pub load_balance_selects: u64,
    pub channel_switches: u64,
    pub total_selections: u64,
}

#[cfg(test)]
impl SchedulerMetrics {
    pub(crate) fn snapshot(&self) -> SchedulerMetricsSnapshot {
        SchedulerMetricsSnapshot {
            sticky_hits: self.inner.sticky_hits.load(Ordering::Relaxed),
            load_balance_selects: self.inner.load_balance_selects.load(Ordering::Relaxed),
            channel_switches: self.inner.channel_switches.load(Ordering::Relaxed),
            total_selections: self.inner.total_selections.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P6.3: 不同 layer 选择更新不同计数
    #[test]
    fn scheduler_metrics_updates_different_counters_per_layer() {
        let metrics = SchedulerMetrics::new();

        metrics.record_sticky_hit();
        metrics.record_sticky_hit();
        metrics.record_load_balance();
        metrics.record_channel_switch();

        let snap = metrics.snapshot();
        assert_eq!(snap.sticky_hits, 2);
        assert_eq!(snap.load_balance_selects, 1);
        assert_eq!(snap.channel_switches, 1);
        assert_eq!(snap.total_selections, 3, "total = sticky + load_balance");
    }

    /// P6.3: 初始状态全部为零
    #[test]
    fn scheduler_metrics_starts_at_zero() {
        let metrics = SchedulerMetrics::new();
        let snap = metrics.snapshot();
        assert_eq!(
            snap,
            SchedulerMetricsSnapshot {
                sticky_hits: 0,
                load_balance_selects: 0,
                channel_switches: 0,
                total_selections: 0,
            }
        );
    }

    /// P6.3: clone 共享同一底层计数器
    #[test]
    fn scheduler_metrics_clone_shares_state() {
        let metrics = SchedulerMetrics::new();
        let cloned = metrics.clone();

        metrics.record_sticky_hit();
        let snap = cloned.snapshot();
        assert_eq!(
            snap.sticky_hits, 1,
            "clone should see updates from original"
        );
    }
}
