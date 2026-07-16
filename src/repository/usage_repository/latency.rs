//! latency 统计：延迟百分位（p50/p95/p99）。

use super::SqliteUsageRepository;

pub(super) async fn get_latency_percentiles(
    repo: &SqliteUsageRepository,
    days: i32,
) -> Result<(Option<f64>, Option<f64>, Option<f64>), sqlx::Error> {
    let (utc_start, utc_end) = repo.range_utc_days(days);

    let latencies: Vec<i32> = sqlx::query_scalar(
        "SELECT latency_ms FROM usage_logs \
         WHERE latency_ms IS NOT NULL AND latency_ms > 0 \
         AND created_at >= ? AND created_at < ? \
         ORDER BY latency_ms ASC",
    )
    .bind(utc_start)
    .bind(utc_end)
    .fetch_all(&repo.pool)
    .await?;

    Ok(compute_percentiles(&latencies))
}

/// 从有序延迟列表计算 p50/p95/p99
fn compute_percentiles(sorted: &[i32]) -> (Option<f64>, Option<f64>, Option<f64>) {
    if sorted.is_empty() {
        return (None, None, None);
    }
    let p = |pct: f64| -> f64 {
        let idx = ((sorted.len() - 1) as f64 * pct).round() as usize;
        sorted[idx.min(sorted.len() - 1)] as f64
    };
    (Some(p(0.50)), Some(p(0.95)), Some(p(0.99)))
}
