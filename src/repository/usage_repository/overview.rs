//! overview 统计：历史总计 + 今日切片。

use super::SqliteUsageRepository;
use crate::domain::usage::StatsOverview;

pub(super) async fn get_overview(repo: &SqliteUsageRepository) -> Result<StatsOverview, sqlx::Error> {
    let total: (i64, i64, i64, f64) = sqlx::query_as(
        "SELECT
                COUNT(*),
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                CAST(COALESCE(SUM(COALESCE(cost, 0)), 0.0) AS REAL)
            FROM usage_logs",
    )
    .fetch_one(&repo.pool)
    .await?;

    let (utc_start, utc_end) = repo.range_utc_days(1);
    let today_stats: (i64, i64, i64, f64) = sqlx::query_as(
        "SELECT
                COUNT(*),
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                CAST(COALESCE(SUM(COALESCE(cost, 0)), 0.0) AS REAL)
            FROM usage_logs
            WHERE created_at >= ? AND created_at < ?",
    )
    .bind(utc_start)
    .bind(utc_end)
    .fetch_one(&repo.pool)
    .await?;

    Ok(StatsOverview {
        total_requests: total.0,
        total_input_tokens: total.1,
        total_output_tokens: total.2,
        total_cost: total.3,
        today_requests: today_stats.0,
        today_input_tokens: today_stats.1,
        today_output_tokens: today_stats.2,
        today_cost: today_stats.3,
        latency_p50: None,
        latency_p95: None,
        latency_p99: None,
    })
}
