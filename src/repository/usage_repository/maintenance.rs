//! 维护查询：proxy 预算门（月/日消费，UTC）+ 保留清理（删 N 天前日志）。

use super::SqliteUsageRepository;

pub(super) async fn aggregate_cost(
    repo: &SqliteUsageRepository,
    api_key_id: &str,
) -> Result<(f64, f64), sqlx::Error> {
    let (monthly, daily): (f64, f64) = sqlx::query_as(
        r#"SELECT
                CAST(COALESCE(SUM(CASE WHEN strftime('%Y-%m', created_at) = strftime('%Y-%m', 'now') THEN COALESCE(cost, 0) ELSE 0 END), 0) AS REAL),
                CAST(COALESCE(SUM(CASE WHEN date(created_at) = date('now') THEN COALESCE(cost, 0) ELSE 0 END), 0) AS REAL)
            FROM usage_logs WHERE api_key_id = ?"#,
    )
    .bind(api_key_id)
    .fetch_one(&repo.pool)
    .await?;
    Ok((monthly, daily))
}

pub(super) async fn delete_older_than(repo: &SqliteUsageRepository, days: i64) -> Result<u64, sqlx::Error> {
    let result = sqlx::query("DELETE FROM usage_logs WHERE created_at < datetime('now', ?)")
        .bind(format!("-{} days", days))
        .execute(&repo.pool)
        .await?;
    Ok(result.rows_affected())
}

/// 清理过期 usage_payloads（请求/响应原文），**保留 usage_logs 统计行**。
/// 分层 retention：content 短期清（释放空间），统计长期保。
pub(super) async fn delete_payloads_older_than(
    repo: &SqliteUsageRepository,
    days: i64,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query("DELETE FROM usage_payloads WHERE created_at < datetime('now', ?)")
        .bind(format!("-{} days", days))
        .execute(&repo.pool)
        .await?;
    Ok(result.rows_affected())
}
