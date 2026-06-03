use std::sync::Arc;
use tokio::time::{Duration, interval};

use super::ratelimit::RateLimiter;
use super::state::LoadBalancerState;

/// 定时任务调度器
pub struct Scheduler {
    lb_state: LoadBalancerState,
    rate_limiter: RateLimiter,
    pool: sqlx::SqlitePool,
}

impl Scheduler {
    pub fn new(lb_state: LoadBalancerState, rate_limiter: RateLimiter, pool: sqlx::SqlitePool) -> Self {
        Self { lb_state, rate_limiter, pool }
    }

    /// 启动定时任务
    pub fn start(self: Arc<Self>) {
        let scheduler = self.clone();
        tokio::spawn(async move {
            scheduler.run_cleanup().await;
        });

        let scheduler = self.clone();
        tokio::spawn(async move {
            scheduler.run_log_cleanup().await;
        });
    }

    /// 清理任务
    async fn run_cleanup(&self) {
        let mut interval = interval(Duration::from_secs(60));

        loop {
            interval.tick().await;

            self.lb_state.cleanup_expired_sessions().await;
            self.lb_state.cleanup_expired_blacklists().await;
            self.rate_limiter.cleanup().await;
        }
    }

    /// 清理超过 90 天的请求日志（每天执行一次）
    async fn run_log_cleanup(&self) {
        let mut interval = interval(Duration::from_secs(86400));

        loop {
            interval.tick().await;

            let result = sqlx::query(
                "DELETE FROM usage_logs WHERE created_at < datetime('now', '-90 days')",
            )
            .execute(&self.pool)
            .await;

            match result {
                Ok(r) => {
                    let deleted = r.rows_affected();
                    if deleted > 0 {
                        tracing::info!("清理了 {} 条过期请求日志（>90天）", deleted);
                    }
                }
                Err(e) => {
                    tracing::warn!("清理请求日志失败: {}", e);
                }
            }
        }
    }
}
