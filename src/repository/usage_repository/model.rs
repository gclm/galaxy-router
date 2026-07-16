//! model 统计：按模型聚合（按天数 / 按日期范围）。

use super::SqliteUsageRepository;
use crate::domain::usage::ModelStats;

pub(super) async fn get_model_stats(
    repo: &SqliteUsageRepository,
    days: i32,
) -> Result<Vec<ModelStats>, sqlx::Error> {
    let (utc_start, utc_end) = repo.range_utc_days(days);
    let stats = sqlx::query_as::<_, (String, i32, i32, i32, f64)>(
        "SELECT
                requested_model,
                COUNT(*),
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                CAST(COALESCE(SUM(COALESCE(cost, 0)), 0.0) AS REAL)
            FROM usage_logs
            WHERE created_at >= ? AND created_at < ?
            GROUP BY requested_model
            ORDER BY COUNT(*) DESC",
    )
    .bind(utc_start)
    .bind(utc_end)
    .fetch_all(&repo.pool)
    .await?;

    Ok(stats
        .into_iter()
        .map(|(model, requests, input, output, cost)| ModelStats {
            model,
            request_count: requests,
            input_tokens: input,
            output_tokens: output,
            total_cost: cost,
        })
        .collect())
}

pub(super) async fn get_model_stats_by_range(
    repo: &SqliteUsageRepository,
    start: &str,
    end: &str,
) -> Result<Vec<ModelStats>, sqlx::Error> {
    let (utc_start, utc_end) = repo.range_utc_between(start, end);
    let stats = sqlx::query_as::<_, (String, i32, i32, i32, f64)>(
        "SELECT
                requested_model,
                COUNT(*),
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                CAST(COALESCE(SUM(COALESCE(cost, 0)), 0.0) AS REAL)
            FROM usage_logs
            WHERE created_at >= ? AND created_at < ?
            GROUP BY requested_model
            ORDER BY COUNT(*) DESC",
    )
    .bind(utc_start)
    .bind(utc_end)
    .fetch_all(&repo.pool)
    .await?;

    Ok(stats
        .into_iter()
        .map(|(model, requests, input, output, cost)| ModelStats {
            model,
            request_count: requests,
            input_tokens: input,
            output_tokens: output,
            total_cost: cost,
        })
        .collect())
}
