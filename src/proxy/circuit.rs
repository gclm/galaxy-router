use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// 熔断器状态
#[derive(Debug, Clone, PartialEq)]
pub enum CircuitState {
    /// 正常通行
    Closed,
    /// 熔断中，拒绝所有请求
    Open,
    /// 半开，仅允许单个试探请求
    HalfOpen,
}

/// 熔断器条目
#[derive(Debug, Clone)]
pub struct CircuitEntry {
    pub state: CircuitState,
    pub consecutive_failures: u64,
    pub last_failure_time: Option<Instant>,
    pub trip_count: u32,
    pub half_open_probe: bool,
}

/// 熔断器配置
#[derive(Debug, Clone)]
pub struct CircuitConfig {
    /// 触发熔断的连续失败次数
    pub failure_threshold: u64,
    /// 基础冷却时间（秒）
    pub base_cooldown_secs: u64,
    /// 最大冷却时间（秒）
    pub max_cooldown_secs: u64,
    /// HalfOpen 状态试探超时（秒）
    pub probe_timeout_secs: u64,
}

impl Default for CircuitConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            base_cooldown_secs: 60,
            max_cooldown_secs: 600,
            probe_timeout_secs: 30,
        }
    }
}

/// 熔断器键：channel_id:key_hint
type CircuitKey = String;

/// 全局熔断器存储
#[derive(Clone)]
pub struct CircuitBreaker {
    entries: Arc<RwLock<HashMap<CircuitKey, CircuitEntry>>>,
    config: CircuitConfig,
}

impl CircuitBreaker {
    pub fn new(config: CircuitConfig) -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            config,
        }
    }

    /// 计算冷却时间（指数退避）
    pub fn cooldown_duration(&self, trip_count: u32) -> Duration {
        if trip_count == 0 {
            return Duration::from_secs(self.config.base_cooldown_secs);
        }

        // base * 2^(trip_count - 1)，防止溢出
        let shift = (trip_count - 1).min(20);
        let cooldown = self.config.base_cooldown_secs * (1 << shift);
        let cooldown = cooldown.min(self.config.max_cooldown_secs);

        Duration::from_secs(cooldown)
    }

    /// 检查渠道是否被熔断
    /// 返回 (is_tripped, remaining_cooldown)
    pub async fn is_tripped(
        &self,
        channel_id: &str,
        key_hint: &str,
    ) -> (bool, Option<Duration>) {
        let key = format!("{}:{}", channel_id, key_hint);
        let entries = self.entries.read().await;

        let Some(entry) = entries.get(&key) else {
            return (false, None);
        };

        match entry.state {
            CircuitState::Closed => (false, None),
            CircuitState::Open => {
                let cooldown = self.cooldown_duration(entry.trip_count);
                let elapsed = entry
                    .last_failure_time
                    .map(|t| t.elapsed())
                    .unwrap_or(Duration::from_secs(u64::MAX));

                if elapsed >= cooldown {
                    // 冷却时间已过，转为 HalfOpen
                    drop(entries);
                    let mut entries = self.entries.write().await;
                    if let Some(entry) = entries.get_mut(&key) {
                        entry.state = CircuitState::HalfOpen;
                        entry.half_open_probe = false;
                        tracing::info!(
                            "circuit [{}] Open -> HalfOpen (cooldown {:?} elapsed)",
                            key,
                            cooldown
                        );
                    }
                    (false, None)
                } else {
                    (true, Some(cooldown - elapsed))
                }
            }
            CircuitState::HalfOpen => {
                if entry.half_open_probe {
                    // 已有试探请求在进行中，拒绝其他请求
                    (true, None)
                } else {
                    (false, None)
                }
            }
        }
    }

    /// 开始试探请求（HalfOpen -> 标记试探中）
    pub async fn begin_probe(&self, channel_id: &str, key_hint: &str) -> bool {
        let key = format!("{}:{}", channel_id, key_hint);
        let mut entries = self.entries.write().await;

        if let Some(entry) = entries.get_mut(&key)
            && entry.state == CircuitState::HalfOpen
            && !entry.half_open_probe
        {
            entry.half_open_probe = true;
            return true;
        }
        false
    }

    /// 记录成功
    pub async fn record_success(&self, channel_id: &str, key_hint: &str) {
        let key = format!("{}:{}", channel_id, key_hint);
        let mut entries = self.entries.write().await;

        if let Some(entry) = entries.get_mut(&key) {
            if entry.state == CircuitState::HalfOpen {
                tracing::info!("circuit [{}] HalfOpen -> Closed (probe succeeded)", key);
            }
            // 重置全部状态
            entry.state = CircuitState::Closed;
            entry.consecutive_failures = 0;
            entry.trip_count = 0;
            entry.half_open_probe = false;
        }
    }

    /// 记录失败
    pub async fn record_failure(&self, channel_id: &str, key_hint: &str) {
        let key = format!("{}:{}", channel_id, key_hint);
        let mut entries = self.entries.write().await;

        let entry = entries.entry(key.clone()).or_insert_with(|| CircuitEntry {
            state: CircuitState::Closed,
            consecutive_failures: 0,
            last_failure_time: None,
            trip_count: 0,
            half_open_probe: false,
        });

        entry.last_failure_time = Some(Instant::now());

        match entry.state {
            CircuitState::Closed => {
                entry.consecutive_failures += 1;
                if entry.consecutive_failures >= self.config.failure_threshold {
                    entry.state = CircuitState::Open;
                    entry.trip_count += 1;
                    let cooldown = self.cooldown_duration(entry.trip_count);
                    tracing::warn!(
                        "circuit [{}] Closed -> Open (failures={}, trip_count={}, cooldown={:?})",
                        key,
                        entry.consecutive_failures,
                        entry.trip_count,
                        cooldown
                    );
                }
            }
            CircuitState::HalfOpen => {
                // 试探失败，重新进入 Open，trip_count 递增
                entry.state = CircuitState::Open;
                entry.trip_count += 1;
                entry.consecutive_failures = 0;
                entry.half_open_probe = false;
                let cooldown = self.cooldown_duration(entry.trip_count);
                tracing::warn!(
                    "circuit [{}] HalfOpen -> Open (probe failed, trip_count={}, cooldown={:?})",
                    key,
                    entry.trip_count,
                    cooldown
                );
            }
            CircuitState::Open => {
                // 理论上不应在 Open 状态收到失败记录
            }
        }
    }

    /// 清理过期条目
    pub async fn cleanup_expired(&self, max_age: Duration) {
        let mut entries = self.entries.write().await;
        let now = Instant::now();
        entries.retain(|_, entry| {
            entry
                .last_failure_time
                .map(|t| now.duration_since(t) < max_age)
                .unwrap_or(true)
        });
    }

    /// 获取熔断器状态（用于监控）
    pub async fn get_status(&self, channel_id: &str, key_hint: &str) -> Option<CircuitState> {
        let key = format!("{}:{}", channel_id, key_hint);
        let entries = self.entries.read().await;
        entries.get(&key).map(|e| e.state.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cooldown_exponential_backoff() {
        let breaker = CircuitBreaker::new(CircuitConfig {
            base_cooldown_secs: 60,
            max_cooldown_secs: 600,
            ..Default::default()
        });

        // 第1次熔断：60s
        assert_eq!(breaker.cooldown_duration(1), Duration::from_secs(60));
        // 第2次熔断：120s
        assert_eq!(breaker.cooldown_duration(2), Duration::from_secs(120));
        // 第3次熔断：240s
        assert_eq!(breaker.cooldown_duration(3), Duration::from_secs(240));
        // 第4次熔断：480s
        assert_eq!(breaker.cooldown_duration(4), Duration::from_secs(480));
        // 第5次熔断：600s（上限）
        assert_eq!(breaker.cooldown_duration(5), Duration::from_secs(600));
        // 第6次熔断：600s（上限）
        assert_eq!(breaker.cooldown_duration(6), Duration::from_secs(600));
    }

    #[test]
    fn test_cooldown_zero_trip_count() {
        let breaker = CircuitBreaker::new(CircuitConfig {
            base_cooldown_secs: 60,
            max_cooldown_secs: 600,
            ..Default::default()
        });

        assert_eq!(breaker.cooldown_duration(0), Duration::from_secs(60));
    }

    #[tokio::test]
    async fn test_circuit_closed_to_open() {
        let breaker = CircuitBreaker::new(CircuitConfig {
            failure_threshold: 3,
            ..Default::default()
        });

        // 初始状态：Closed
        let (tripped, _) = breaker.is_tripped("ch1", "key1").await;
        assert!(!tripped);

        // 记录失败
        breaker.record_failure("ch1", "key1").await;
        breaker.record_failure("ch1", "key1").await;

        // 还没达到阈值
        let (tripped, _) = breaker.is_tripped("ch1", "key1").await;
        assert!(!tripped);

        // 达到阈值，转为 Open
        breaker.record_failure("ch1", "key1").await;
        let (tripped, remaining) = breaker.is_tripped("ch1", "key1").await;
        assert!(tripped);
        assert!(remaining.is_some());
    }

    #[tokio::test]
    async fn test_circuit_success_resets() {
        let breaker = CircuitBreaker::new(CircuitConfig {
            failure_threshold: 3,
            ..Default::default()
        });

        // 触发熔断
        for _ in 0..3 {
            breaker.record_failure("ch1", "key1").await;
        }
        let (tripped, _) = breaker.is_tripped("ch1", "key1").await;
        assert!(tripped);

        // 成功请求重置状态
        breaker.record_success("ch1", "key1").await;
        let (tripped, _) = breaker.is_tripped("ch1", "key1").await;
        assert!(!tripped);
    }

    #[tokio::test]
    async fn test_circuit_half_open_probe() {
        let breaker = CircuitBreaker::new(CircuitConfig {
            failure_threshold: 2,
            base_cooldown_secs: 0, // 立即冷却
            ..Default::default()
        });

        // 触发熔断
        breaker.record_failure("ch1", "key1").await;
        breaker.record_failure("ch1", "key1").await;

        // 冷却时间已过，转为 HalfOpen
        let (tripped, _) = breaker.is_tripped("ch1", "key1").await;
        assert!(!tripped);

        // 开始试探
        let probe_started = breaker.begin_probe("ch1", "key1").await;
        assert!(probe_started);

        // 试探期间拒绝其他请求
        let (tripped, _) = breaker.is_tripped("ch1", "key1").await;
        assert!(tripped);

        // 试探成功，转为 Closed
        breaker.record_success("ch1", "key1").await;
        let (tripped, _) = breaker.is_tripped("ch1", "key1").await;
        assert!(!tripped);
    }

    #[tokio::test]
    async fn test_circuit_half_open_probe_failure() {
        let breaker = CircuitBreaker::new(CircuitConfig {
            failure_threshold: 2,
            base_cooldown_secs: 60, // 使用非零冷却时间
            ..Default::default()
        });

        // 触发熔断
        breaker.record_failure("ch1", "key1").await;
        breaker.record_failure("ch1", "key1").await;

        // 验证熔断状态
        let (tripped, remaining) = breaker.is_tripped("ch1", "key1").await;
        assert!(tripped);
        assert!(remaining.is_some());

        // 验证状态是 Open
        let status = breaker.get_status("ch1", "key1").await;
        assert_eq!(status, Some(CircuitState::Open));

        // 模拟冷却时间已过（手动修改 last_failure_time）
        {
            let mut entries = breaker.entries.write().await;
            if let Some(entry) = entries.get_mut("ch1:key1") {
                entry.last_failure_time = Some(
                    std::time::Instant::now() - std::time::Duration::from_secs(120),
                );
            }
        }

        // 冷却时间已过，转为 HalfOpen
        let (tripped, _) = breaker.is_tripped("ch1", "key1").await;
        assert!(!tripped);

        // 验证状态是 HalfOpen
        let status = breaker.get_status("ch1", "key1").await;
        assert_eq!(status, Some(CircuitState::HalfOpen));

        // 开始试探
        let probe_started = breaker.begin_probe("ch1", "key1").await;
        assert!(probe_started);

        // 试探失败，重新进入 Open
        breaker.record_failure("ch1", "key1").await;

        // 验证状态是 Open（trip_count 递增）
        let status = breaker.get_status("ch1", "key1").await;
        assert_eq!(status, Some(CircuitState::Open));

        // 验证仍然熔断
        let (tripped, _) = breaker.is_tripped("ch1", "key1").await;
        assert!(tripped);
    }

    #[tokio::test]
    async fn test_circuit_different_keys() {
        let breaker = CircuitBreaker::new(CircuitConfig {
            failure_threshold: 2,
            ..Default::default()
        });

        // ch1 熔断
        breaker.record_failure("ch1", "key1").await;
        breaker.record_failure("ch1", "key1").await;
        let (tripped, _) = breaker.is_tripped("ch1", "key1").await;
        assert!(tripped);

        // ch2 不受影响
        let (tripped, _) = breaker.is_tripped("ch2", "key1").await;
        assert!(!tripped);
    }

    #[tokio::test]
    async fn test_cleanup_expired() {
        let breaker = CircuitBreaker::new(CircuitConfig {
            failure_threshold: 2,
            ..Default::default()
        });

        // 触发熔断
        breaker.record_failure("ch1", "key1").await;
        breaker.record_failure("ch1", "key1").await;
        let (tripped, _) = breaker.is_tripped("ch1", "key1").await;
        assert!(tripped);

        // 清理过期条目（0 秒前的都算过期）
        breaker.cleanup_expired(Duration::from_secs(0)).await;

        // 清理后状态重置
        let (tripped, _) = breaker.is_tripped("ch1", "key1").await;
        assert!(!tripped);
    }
}
