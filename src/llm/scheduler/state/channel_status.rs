//! 单渠道运行时状态。

use chrono::{DateTime, Utc};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tokio::sync::RwLock;

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
    /// 渠道级黑名单触发阈值（连续失败次数）。来自 channels.failure_threshold
    pub blacklist_threshold: u64,
    /// 渠道级黑名单时长（分钟）。来自 channels.blacklist_minutes
    pub blacklist_minutes: i64,
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
            // 默认值，ensure_channel_status 会用渠道配置覆盖
            blacklist_threshold: 3,
            blacklist_minutes: 10,
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

        // 失败计数衰减：
        //   1) 若距上次失败超过 30 分钟（TTL），直接清零（参考 axonhub FailureStatsTTL）
        //   2) 否则折半，避免单次成功洗白
        const FAILURE_TTL: chrono::TimeDelta = chrono::TimeDelta::minutes(30);
        let ttl_expired = self
            .last_failure
            .map(|t| Utc::now() - t >= FAILURE_TTL)
            .unwrap_or(true);
        if ttl_expired {
            self.failure_count = 0;
        } else {
            self.failure_count = self.failure_count.saturating_div(2);
        }

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

    /// 记录失败（带 TTL 衰减）
    ///
    /// 若距上次失败超过 30 分钟，计数器重置为 1 再递增，避免历史 sporadic 失败累积导致渠道黑名单。
    pub fn record_failure(&mut self) {
        const FAILURE_TTL: chrono::TimeDelta = chrono::TimeDelta::minutes(30);
        let ttl_expired = self
            .last_failure
            .map(|t| Utc::now() - t >= FAILURE_TTL)
            .unwrap_or(true);
        if ttl_expired {
            self.failure_count = 0;
        }
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
        // TTL 依赖 last_failure：设定为刚刚，避免 30min 衰减把计数清零
        s.last_failure = Some(Utc::now());
        s.record_success(100.0).await;
        assert_eq!(s.failure_count, 2);
        s.record_success(100.0).await;
        assert_eq!(s.failure_count, 1);
        s.record_success(100.0).await;
        assert_eq!(s.failure_count, 0);
    }

    #[tokio::test]
    async fn record_success_clears_failure_when_ttl_expired() {
        // 30 分钟内无失败 → 单次成功直接清零（axonhub FailureStatsTTL 模式）
        let mut s = sample_status();
        s.failure_count = 5;
        s.last_failure = Some(Utc::now() - chrono::TimeDelta::minutes(31));
        s.record_success(100.0).await;
        assert_eq!(s.failure_count, 0);
    }

    #[tokio::test]
    async fn record_failure_resets_after_ttl_gap() {
        // 两次失败间隔 > 30 分钟 → 第二次失败从 1 开始计，避免历史累积
        let mut s = sample_status();
        s.failure_count = 5;
        s.last_failure = Some(Utc::now() - chrono::TimeDelta::minutes(31));
        s.record_failure();
        assert_eq!(s.failure_count, 1);
        // 短间隔内的后续失败正常累积
        s.record_failure();
        assert_eq!(s.failure_count, 2);
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
