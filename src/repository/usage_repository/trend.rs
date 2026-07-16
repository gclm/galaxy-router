//! trend 统计：按天/按小时趋势（days<=1 时按小时补齐 24 槽）。

use sqlx::AssertSqlSafe;

use super::{DailyRow, SqliteUsageRepository, daily_row_to_stats};
use crate::domain::usage::DailyStats;

pub(super) async fn get_daily_stats(
    repo: &SqliteUsageRepository,
    days: i32,
) -> Result<Vec<DailyStats>, sqlx::Error> {
    let tz = repo.tz_modifier();

    if days <= 1 {
        let (utc_start, utc_end) = repo.range_utc_days(1);
        let sql = format!(
            "SELECT
                    strftime('%H:00', datetime(created_at, '{}')),
                    COUNT(*),
                    SUM(CASE WHEN status_code >= 200 AND status_code < 400 THEN 1 ELSE 0 END),
                    SUM(CASE WHEN status_code < 200 OR status_code >= 400 THEN 1 ELSE 0 END),
                    COALESCE(SUM(input_tokens), 0),
                    COALESCE(SUM(output_tokens), 0),
                    COALESCE(SUM(cache_read_tokens), 0),
                    COALESCE(SUM(cache_creation_tokens), 0),
                    CAST(COALESCE(SUM(COALESCE(cost, 0)), 0.0) AS REAL)
                FROM usage_logs
                WHERE created_at >= ? AND created_at < ?
                GROUP BY strftime('%H', datetime(created_at, '{}'))
                ORDER BY strftime('%H', datetime(created_at, '{}')) ASC",
            tz, tz, tz
        );
        let rows: Vec<DailyRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(utc_start)
            .bind(utc_end)
            .fetch_all(&repo.pool)
            .await?;

        return Ok(repo.fill_hourly(rows));
    }

    let (utc_start, utc_end) = repo.range_utc_days(days);

    let sql = format!(
        "SELECT
                date(datetime(created_at, '{}')),
                COUNT(*),
                SUM(CASE WHEN status_code >= 200 AND status_code < 400 THEN 1 ELSE 0 END),
                SUM(CASE WHEN status_code < 200 OR status_code >= 400 THEN 1 ELSE 0 END),
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                COALESCE(SUM(cache_read_tokens), 0),
                COALESCE(SUM(cache_creation_tokens), 0),
                CAST(COALESCE(SUM(COALESCE(cost, 0)), 0.0) AS REAL)
            FROM usage_logs
            WHERE created_at >= ? AND created_at < ?
            GROUP BY date(datetime(created_at, '{}'))
            ORDER BY date(datetime(created_at, '{}')) ASC",
        tz, tz, tz
    );
    let rows: Vec<DailyRow> = sqlx::query_as(AssertSqlSafe(sql))
        .bind(utc_start)
        .bind(utc_end)
        .fetch_all(&repo.pool)
        .await?;

    Ok(rows.into_iter().map(daily_row_to_stats).collect())
}

pub(super) async fn get_daily_stats_by_range(
    repo: &SqliteUsageRepository,
    start: &str,
    end: &str,
) -> Result<Vec<DailyStats>, sqlx::Error> {
    let tz = repo.tz_modifier();
    let (utc_start, utc_end) = repo.range_utc_between(start, end);
    let sql = format!(
        "SELECT
                date(datetime(created_at, '{}')),
                COUNT(*),
                SUM(CASE WHEN status_code >= 200 AND status_code < 400 THEN 1 ELSE 0 END),
                SUM(CASE WHEN status_code < 200 OR status_code >= 400 THEN 1 ELSE 0 END),
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                COALESCE(SUM(cache_read_tokens), 0),
                COALESCE(SUM(cache_creation_tokens), 0),
                CAST(COALESCE(SUM(COALESCE(cost, 0)), 0.0) AS REAL)
            FROM usage_logs
            WHERE created_at >= ? AND created_at < ?
            GROUP BY date(datetime(created_at, '{}'))
            ORDER BY date(datetime(created_at, '{}')) ASC",
        tz, tz, tz
    );
    let rows: Vec<DailyRow> = sqlx::query_as(AssertSqlSafe(sql))
        .bind(utc_start)
        .bind(utc_end)
        .fetch_all(&repo.pool)
        .await?;

    Ok(rows.into_iter().map(daily_row_to_stats).collect())
}
