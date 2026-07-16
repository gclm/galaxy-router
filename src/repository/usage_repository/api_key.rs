//! api_key 统计：按 Key 聚合（含成功率/均延迟）。

use super::SqliteUsageRepository;
use crate::domain::usage::ApiKeyStats;

pub(super) async fn get_api_key_stats(
    repo: &SqliteUsageRepository,
    days: i32,
) -> Result<Vec<ApiKeyStats>, sqlx::Error> {
    let cutoff = repo.now_local() - chrono::Duration::days(days as i64);
    let cutoff_str = cutoff.format("%Y-%m-%d %H:%M:%S").to_string();

    let rows = sqlx::query_as::<_, (String, Option<String>, i32, i32, i32, i32, i32, f64, f64)>(
        r#"
            SELECT
                ul.api_key_id,
                ak.name AS api_key_name,
                COUNT(*) AS request_count,
                SUM(CASE WHEN ul.status_code >= 200 AND ul.status_code < 400 THEN 1 ELSE 0 END) AS success_count,
                SUM(CASE WHEN ul.status_code IS NULL OR ul.status_code < 200 OR ul.status_code >= 400 THEN 1 ELSE 0 END) AS failure_count,
                COALESCE(SUM(ul.input_tokens), 0) AS input_tokens,
                COALESCE(SUM(ul.output_tokens), 0) AS output_tokens,
                COALESCE(SUM(ul.cost), 0.0) AS total_cost,
                COALESCE(AVG(CAST(ul.latency_ms AS REAL)), 0.0) AS avg_latency_ms
            FROM usage_logs ul
            LEFT JOIN api_keys ak ON ul.api_key_id = ak.id
            WHERE ul.api_key_id IS NOT NULL AND ul.created_at >= ?
            GROUP BY ul.api_key_id
            ORDER BY total_cost DESC
            "#,
    )
    .bind(&cutoff_str)
    .fetch_all(&repo.pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(
                api_key_id,
                api_key_name,
                request_count,
                success_count,
                failure_count,
                input_tokens,
                output_tokens,
                total_cost,
                avg_latency_ms,
            )| {
                ApiKeyStats {
                    api_key_id,
                    api_key_name,
                    request_count,
                    success_count,
                    failure_count,
                    input_tokens,
                    output_tokens,
                    total_cost,
                    avg_latency_ms,
                }
            },
        )
        .collect())
}
