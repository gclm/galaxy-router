//! channel 统计：按渠道聚合（LEFT JOIN 渠道名，按天数 / 按日期范围）。

use super::SqliteUsageRepository;
use crate::domain::usage::ChannelStats;

/// 渠道统计行类型
type ChannelStatsRow = (String, String, i32, i32, i32, i32, i32, f64);

pub(super) async fn get_channel_stats(
    repo: &SqliteUsageRepository,
    days: i32,
) -> Result<Vec<ChannelStats>, sqlx::Error> {
    let (utc_start, utc_end) = repo.range_utc_days(days);
    let rows: Vec<ChannelStatsRow> = sqlx::query_as(
        "SELECT
                ul.channel_id,
                COALESCE(c.name, 'unknown'),
                COUNT(*),
                SUM(CASE WHEN ul.status_code >= 200 AND ul.status_code < 400 THEN 1 ELSE 0 END),
                SUM(CASE WHEN ul.status_code < 200 OR ul.status_code >= 400 THEN 1 ELSE 0 END),
                COALESCE(SUM(ul.input_tokens), 0),
                COALESCE(SUM(ul.output_tokens), 0),
                CAST(COALESCE(SUM(COALESCE(ul.cost, 0)), 0.0) AS REAL)
            FROM usage_logs ul
            LEFT JOIN channels c ON ul.channel_id = c.id
            WHERE ul.created_at >= ? AND ul.created_at < ?
            GROUP BY ul.channel_id
            ORDER BY COUNT(*) DESC",
    )
    .bind(utc_start)
    .bind(utc_end)
    .fetch_all(&repo.pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(id, name, requests, success, failure, input, output, cost)| ChannelStats {
                channel_id: id,
                channel_name: name,
                request_count: requests,
                success_count: success,
                failure_count: failure,
                input_tokens: input,
                output_tokens: output,
                total_cost: cost,
            },
        )
        .collect())
}

pub(super) async fn get_channel_stats_by_range(
    repo: &SqliteUsageRepository,
    start: &str,
    end: &str,
) -> Result<Vec<ChannelStats>, sqlx::Error> {
    let (utc_start, utc_end) = repo.range_utc_between(start, end);
    let rows: Vec<ChannelStatsRow> = sqlx::query_as(
        "SELECT
                ul.channel_id,
                COALESCE(c.name, 'unknown'),
                COUNT(*),
                SUM(CASE WHEN ul.status_code >= 200 AND ul.status_code < 400 THEN 1 ELSE 0 END),
                SUM(CASE WHEN ul.status_code < 200 OR ul.status_code >= 400 THEN 1 ELSE 0 END),
                COALESCE(SUM(ul.input_tokens), 0),
                COALESCE(SUM(ul.output_tokens), 0),
                CAST(COALESCE(SUM(COALESCE(ul.cost, 0)), 0.0) AS REAL)
            FROM usage_logs ul
            LEFT JOIN channels c ON ul.channel_id = c.id
            WHERE ul.created_at >= ? AND ul.created_at < ?
            GROUP BY ul.channel_id
            ORDER BY COUNT(*) DESC",
    )
    .bind(utc_start)
    .bind(utc_end)
    .fetch_all(&repo.pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(id, name, requests, success, failure, input, output, cost)| ChannelStats {
                channel_id: id,
                channel_name: name,
                request_count: requests,
                success_count: success,
                failure_count: failure,
                input_tokens: input,
                output_tokens: output,
                total_cost: cost,
            },
        )
        .collect())
}
