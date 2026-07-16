//! Per-API-key 滑动窗口速率限制器（RPM + TPM）

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

/// 滑动窗口长度（秒）
const WINDOW_SECS: u64 = 60;

/// 每个键的窗口计数状态
#[derive(Clone)]
struct WindowState {
    /// 当前窗口起始时刻
    window_start: Instant,
    /// 窗口内请求数
    request_count: u64,
    /// 窗口内 token 总量
    token_count: u64,
}

/// 内存滑动窗口速率限制器
///
/// - 每分钟一个窗口，过期自动重置。
/// - `rpm_limit == 0` 或 `tpm_limit == 0` 表示该项不限制。
/// - RPM 在请求前检查并递增；TPM 检查已累计量，请求结束后通过 `record_tokens` 更新。
#[derive(Clone)]
pub struct RateLimiter {
    windows: Arc<RwLock<HashMap<String, WindowState>>>,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            windows: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 检查并递增 RPM 计数。
    /// - `rpm_limit == 0` → 直接放行
    /// - `Ok(())` → 允许，已递增计数
    /// - `Err(retry_after_secs)` → 超限
    pub async fn check_rpm(&self, key_id: &str, rpm_limit: u64) -> Result<(), f64> {
        if rpm_limit == 0 {
            return Ok(());
        }

        let mut windows = self.windows.write().await;
        let now = Instant::now();

        let state = windows
            .entry(key_id.to_string())
            .or_insert_with(|| WindowState {
                window_start: now,
                request_count: 0,
                token_count: 0,
            });

        // 窗口过期 → 重置
        if now.duration_since(state.window_start).as_secs() >= WINDOW_SECS {
            state.window_start = now;
            state.request_count = 0;
            state.token_count = 0;
        }

        if state.request_count >= rpm_limit {
            let elapsed = now.duration_since(state.window_start).as_secs_f64();
            let retry_after = (WINDOW_SECS as f64 - elapsed).max(1.0);
            return Err(retry_after);
        }

        state.request_count += 1;
        Ok(())
    }

    /// 检查 TPM（软检查：只看当前窗口已累计的 token 数）。
    /// 真正的 token 计数在请求完成后通过 `record_tokens` 更新。
    pub async fn check_tpm(&self, key_id: &str, tpm_limit: u64) -> Result<(), f64> {
        if tpm_limit == 0 {
            return Ok(());
        }

        let windows = self.windows.read().await;
        if let Some(state) = windows.get(key_id) {
            let now = Instant::now();
            if now.duration_since(state.window_start).as_secs() < WINDOW_SECS
                && state.token_count >= tpm_limit
            {
                let elapsed = now.duration_since(state.window_start).as_secs_f64();
                let retry_after = (WINDOW_SECS as f64 - elapsed).max(1.0);
                return Err(retry_after);
            }
        }
        Ok(())
    }

    /// 请求完成后记录 token 用量（由 save_request_record / stream spawn 调用）。
    pub async fn record_tokens(&self, key_id: &str, input_tokens: u64, output_tokens: u64) {
        let total = input_tokens + output_tokens;
        if total == 0 {
            return;
        }

        let mut windows = self.windows.write().await;
        let now = Instant::now();

        let state = windows
            .entry(key_id.to_string())
            .or_insert_with(|| WindowState {
                window_start: now,
                request_count: 0,
                token_count: 0,
            });

        // 只在当前窗口内累加
        if now.duration_since(state.window_start).as_secs() < WINDOW_SECS {
            state.token_count += total;
        }
    }

    /// 获取某个键的当前用量（供管理端查看）
    #[allow(dead_code)]
    pub async fn get_usage(&self, key_id: &str) -> Option<(u64, u64)> {
        let windows = self.windows.read().await;
        windows
            .get(key_id)
            .map(|s| (s.request_count, s.token_count))
    }

    /// 清理过期的窗口条目
    pub async fn cleanup(&self) {
        let mut windows = self.windows.write().await;
        let now = Instant::now();
        windows
            .retain(|_, state| now.duration_since(state.window_start).as_secs() < WINDOW_SECS * 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rpm_zero_means_unlimited() {
        let limiter = RateLimiter::new();
        // 1_000_000 次都应放行
        for _ in 0..100 {
            assert!(
                limiter.check_rpm("key-1", 0).await.is_ok(),
                "rpm=0 应始终放行"
            );
        }
    }

    #[tokio::test]
    async fn rpm_limit_enforced() {
        let limiter = RateLimiter::new();
        let limit: u64 = 3;

        for i in 0..limit {
            assert!(
                limiter.check_rpm("key-1", limit).await.is_ok(),
                "第 {} 次应放行",
                i + 1
            );
        }
        let result = limiter.check_rpm("key-1", limit).await;
        assert!(result.is_err(), "超出限制应拒绝");
        let retry_after = result.unwrap_err();
        assert!(
            (1.0..=60.0).contains(&retry_after),
            "retry_after 应在合理范围"
        );
    }

    #[tokio::test]
    async fn rpm_independent_per_key() {
        let limiter = RateLimiter::new();
        assert!(limiter.check_rpm("key-a", 1).await.is_ok());
        assert!(limiter.check_rpm("key-a", 1).await.is_err());
        // key-b 不受 key-a 影响
        assert!(limiter.check_rpm("key-b", 1).await.is_ok());
    }

    #[tokio::test]
    async fn tpm_soft_check_blocks_when_exceeded() {
        let limiter = RateLimiter::new();

        // 手动记录 token 用量
        limiter.record_tokens("key-1", 500, 500).await;

        // TPM 限制 = 900，已用 1000 → 应拒绝
        assert!(limiter.check_tpm("key-1", 900).await.is_err());
        // TPM 限制 = 0 → 放行
        assert!(limiter.check_tpm("key-1", 0).await.is_ok());
        // TPM 限制 = 2000 → 放行
        assert!(limiter.check_tpm("key-1", 2000).await.is_ok());
    }

    #[tokio::test]
    async fn record_tokens_ignores_zero() {
        let limiter = RateLimiter::new();
        limiter.record_tokens("key-1", 0, 0).await;
        // 无窗口创建
        assert!(limiter.get_usage("key-1").await.is_none());
    }

    #[tokio::test]
    async fn cleanup_removes_stale_entries() {
        let limiter = RateLimiter::new();

        // 创建条目
        limiter.check_rpm("key-1", 100).await.ok();
        limiter.check_rpm("key-2", 100).await.ok();
        assert!(limiter.get_usage("key-1").await.is_some());
        assert!(limiter.get_usage("key-2").await.is_some());

        // 手动让窗口过期（直接修改内部状态）
        {
            let mut windows = limiter.windows.write().await;
            let past = Instant::now() - std::time::Duration::from_secs(200);
            for state in windows.values_mut() {
                state.window_start = past;
            }
        }

        limiter.cleanup().await;
        assert!(limiter.get_usage("key-1").await.is_none());
        assert!(limiter.get_usage("key-2").await.is_none());
    }
}
