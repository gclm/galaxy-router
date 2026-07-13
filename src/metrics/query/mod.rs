use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, SqlitePool};
use std::collections::HashMap;

/// 生成 SQLite datetime() 修饰符，如 "+8 hours" 或 "-5 hours"
pub fn tz_modifier(offset: i32) -> String {
    assert!(
        (-12..=14).contains(&offset),
        "时区偏移量超出合理范围: {}",
        offset
    );
    if offset >= 0 {
        format!("+{} hours", offset)
    } else {
        format!("-{} hours", offset.abs())
    }
}

/// 生成当前本地时间字符串（用于 INSERT）
pub fn now_local_str(offset: i32) -> String {
    (chrono::Utc::now() + chrono::Duration::hours(offset as i64))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

/// 统计状态
#[derive(Clone)]
pub struct StatsState {
    pub pool: SqlitePool,
    pub timezone_offset: i32,
}

impl StatsState {
    pub fn new(pool: SqlitePool, timezone_offset: i32) -> Self {
        Self {
            pool,
            timezone_offset,
        }
    }

    fn tz_modifier(&self) -> String {
        tz_modifier(self.timezone_offset)
    }

    fn now_local(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now() + chrono::Duration::hours(self.timezone_offset as i64)
    }

    /// 本地"最近 days 天(含今天)"对应的 UTC 时间范围 [start, end)。
    /// created_at 以 UTC 存储,用裸列 `created_at >= ? AND created_at < ?` 比较才能命中
    /// idx_usage_logs_created_at(避免 `date(datetime(created_at,...))` 让索引失效、全表扫)。
    fn range_utc_days(&self, days: i32) -> (String, String) {
        let tz = self.timezone_offset as i64;
        let today = (chrono::Utc::now() + chrono::Duration::hours(tz)).date_naive();
        let start = today - chrono::Duration::days((days.max(1) - 1) as i64);
        let end = today + chrono::Duration::days(1);
        let to_utc = |d: chrono::NaiveDate| {
            (d.and_hms_opt(0, 0, 0).unwrap() - chrono::Duration::hours(tz))
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        };
        (to_utc(start), to_utc(end))
    }

    /// 本地日期范围 [start_local, end_local](含两端)对应的 UTC 范围 [start, end_next_day)。
    fn range_utc_between(&self, start_local: &str, end_local: &str) -> (String, String) {
        let tz = self.timezone_offset as i64;
        let today = (chrono::Utc::now() + chrono::Duration::hours(tz)).date_naive();
        let start = chrono::NaiveDate::parse_from_str(start_local, "%Y-%m-%d").unwrap_or(today);
        let end =
            chrono::NaiveDate::parse_from_str(end_local, "%Y-%m-%d").unwrap_or(today) + chrono::Duration::days(1);
        let to_utc = |d: chrono::NaiveDate| {
            (d.and_hms_opt(0, 0, 0).unwrap() - chrono::Duration::hours(tz))
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        };
        (to_utc(start), to_utc(end))
    }
}

/// 按日期聚合的统计行
type DailyRow = (String, i32, i32, i32, i32, i32, i32, i32, f64);

/// 渠道统计行类型
type ChannelStatsRow = (String, String, i32, i32, i32, i32, i32, f64);

/// 统计概览
#[derive(Debug, Serialize, Deserialize)]
pub struct StatsOverview {
    pub total_requests: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cost: f64,
    pub today_requests: i64,
    pub today_input_tokens: i64,
    pub today_output_tokens: i64,
    pub today_cost: f64,
    pub latency_p50: Option<f64>,
    pub latency_p95: Option<f64>,
    pub latency_p99: Option<f64>,
}

/// 按模型统计
#[derive(Debug, Serialize, Deserialize)]
pub struct ModelStats {
    pub model: String,
    pub request_count: i32,
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub total_cost: f64,
}

/// 按渠道统计
#[derive(Debug, Serialize, Deserialize)]
pub struct ChannelStats {
    pub channel_id: String,
    pub channel_name: String,
    pub request_count: i32,
    pub success_count: i32,
    pub failure_count: i32,
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub total_cost: f64,
}

/// 每日统计（按天聚合后返回给前端）
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DailyStats {
    pub date: String,
    pub request_count: i32,
    pub success_count: i32,
    pub failure_count: i32,
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub cache_read_tokens: i32,
    pub cache_creation_tokens: i32,
    pub total_cost: f64,
}

/// 按 API Key 统计
#[derive(Debug, Serialize, Deserialize)]
pub struct ApiKeyStats {
    pub api_key_id: String,
    pub api_key_name: Option<String>,
    pub request_count: i32,
    pub success_count: i32,
    pub failure_count: i32,
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub total_cost: f64,
    pub avg_latency_ms: f64,
}

/// 请求日志筛选条件
pub struct LogsFilter {
    pub offset: u32,
    pub limit: u32,
    pub model: Option<String>,
    pub channel_id: Option<String>,
    pub status: Option<String>,
    pub api_key_id: Option<String>,
}

/// 请求日志（含渠道名和 Key 名）
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct UsageLogRow {
    pub id: String,
    pub api_key_id: Option<String>,
    pub api_key_name: Option<String>,
    pub channel_id: Option<String>,
    pub channel_name: Option<String>,
    pub route_id: Option<String>,
    pub requested_model: String,
    pub actual_model: Option<String>,
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub cache_read_tokens: i32,
    pub cache_creation_tokens: i32,
    pub cost: Option<f64>,
    pub latency_ms: Option<i32>,
    pub ttft_ms: Option<i32>,
    pub status_code: Option<i32>,
    pub error_message: Option<String>,
    pub created_at: String,
    pub endpoint_type: Option<String>,
    pub request_type: String,
    pub is_stream: bool,
    pub upstream_key_hint: Option<String>,
    pub user_agent: Option<String>,
    #[sqlx(skip)]
    pub attempts: Option<serde_json::Value>,
    pub raw_attempts: Option<String>,
}

/// 请求日志详情（含请求/响应内容）
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct UsageLogDetail {
    pub id: String,
    pub api_key_id: Option<String>,
    pub api_key_name: Option<String>,
    pub channel_id: Option<String>,
    pub channel_name: Option<String>,
    pub route_id: Option<String>,
    pub requested_model: String,
    pub actual_model: Option<String>,
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub cache_read_tokens: i32,
    pub cache_creation_tokens: i32,
    pub cost: Option<f64>,
    pub latency_ms: Option<i32>,
    pub ttft_ms: Option<i32>,
    pub status_code: Option<i32>,
    pub error_message: Option<String>,
    pub created_at: String,
    pub endpoint_type: Option<String>,
    pub request_type: String,
    pub request_content: Option<String>,
    pub response_content: Option<String>,
    pub is_stream: bool,
    pub upstream_key_hint: Option<String>,
    pub user_agent: Option<String>,
    #[sqlx(skip)]
    pub attempts: Option<serde_json::Value>,
    pub raw_attempts: Option<String>,
}

/// 分页结果
pub struct PagedResult<T> {
    pub items: Vec<T>,
    pub total: i64,
}

impl StatsState {
    pub async fn get_overview(&self) -> Result<StatsOverview, sqlx::Error> {
        let total: (i64, i64, i64, f64) = sqlx::query_as(
            "SELECT
                COUNT(*),
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                CAST(COALESCE(SUM(COALESCE(cost, 0)), 0.0) AS REAL)
            FROM usage_logs",
        )
        .fetch_one(&self.pool)
        .await?;

        let (utc_start, utc_end) = self.range_utc_days(1);
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
        .fetch_one(&self.pool)
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

    /// 计算延迟百分位（p50/p95/p99）
    pub async fn get_latency_percentiles(
        &self,
        days: i32,
    ) -> Result<(Option<f64>, Option<f64>, Option<f64>), sqlx::Error> {
        let (utc_start, utc_end) = self.range_utc_days(days);

        let latencies: Vec<i32> = sqlx::query_scalar(
            "SELECT latency_ms FROM usage_logs \
             WHERE latency_ms IS NOT NULL AND latency_ms > 0 \
             AND created_at >= ? AND created_at < ? \
             ORDER BY latency_ms ASC",
        )
        .bind(utc_start)
        .bind(utc_end)
        .fetch_all(&self.pool)
        .await?;

        Ok(compute_percentiles(&latencies))
    }

    /// 获取按模型统计
    pub async fn get_model_stats(&self, days: i32) -> Result<Vec<ModelStats>, sqlx::Error> {
        let (utc_start, utc_end) = self.range_utc_days(days);
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
        .fetch_all(&self.pool)
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

    /// 获取按渠道统计
    pub async fn get_channel_stats(&self, days: i32) -> Result<Vec<ChannelStats>, sqlx::Error> {
        let (utc_start, utc_end) = self.range_utc_days(days);
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
        .fetch_all(&self.pool)
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

    /// 获取按天统计（days=1 时按小时聚合，补齐 24 小时）
    pub async fn get_daily_stats(&self, days: i32) -> Result<Vec<DailyStats>, sqlx::Error> {
        let tz = self.tz_modifier();

        if days <= 1 {
            let (utc_start, utc_end) = self.range_utc_days(1);
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
                .fetch_all(&self.pool)
                .await?;

            return Ok(self.fill_hourly(rows));
        }

        let (utc_start, utc_end) = self.range_utc_days(days);

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
            .fetch_all(&self.pool)
            .await?;

        Ok(rows.into_iter().map(daily_row_to_stats).collect())
    }

    /// 按日期范围获取按天统计
    pub async fn get_daily_stats_by_range(
        &self,
        start: &str,
        end: &str,
    ) -> Result<Vec<DailyStats>, sqlx::Error> {
        let tz = self.tz_modifier();
        let (utc_start, utc_end) = self.range_utc_between(start, end);
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
            .fetch_all(&self.pool)
            .await?;

        Ok(rows.into_iter().map(daily_row_to_stats).collect())
    }

    /// 按日期范围获取按模型统计
    pub async fn get_model_stats_by_range(
        &self,
        start: &str,
        end: &str,
    ) -> Result<Vec<ModelStats>, sqlx::Error> {
        let (utc_start, utc_end) = self.range_utc_between(start, end);
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
        .fetch_all(&self.pool)
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

    /// 按日期范围获取按渠道统计
    pub async fn get_channel_stats_by_range(
        &self,
        start: &str,
        end: &str,
    ) -> Result<Vec<ChannelStats>, sqlx::Error> {
        let (utc_start, utc_end) = self.range_utc_between(start, end);
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
        .fetch_all(&self.pool)
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

    /// 获取请求日志（分页 + 筛选）
    pub async fn get_logs(
        &self,
        filter: LogsFilter,
    ) -> Result<PagedResult<UsageLogRow>, sqlx::Error> {
        use sqlx::QueryBuilder;

        let mut count_builder = QueryBuilder::new("SELECT COUNT(*) FROM usage_logs ul WHERE 1=1");
        if let Some(ref model) = filter.model {
            count_builder.push(" AND ul.requested_model = ");
            count_builder.push_bind(model.clone());
        }
        if let Some(ref cid) = filter.channel_id {
            count_builder.push(" AND ul.channel_id = ");
            count_builder.push_bind(cid.clone());
        }
        if let Some(ref kid) = filter.api_key_id {
            count_builder.push(" AND ul.api_key_id = ");
            count_builder.push_bind(kid.clone());
        }
        match filter.status.as_deref() {
            Some("success") => {
                count_builder.push(" AND ul.status_code >= 200 AND ul.status_code < 400");
            }
            Some("failure") => {
                count_builder.push(" AND (ul.status_code < 200 OR ul.status_code >= 400)");
            }
            _ => {}
        }

        let total: (i64,) = count_builder.build_query_as().fetch_one(&self.pool).await?;

        let tz = self.tz_modifier();
        let mut data_builder = QueryBuilder::new(format!(
            r#"SELECT ul.id, ul.api_key_id, ak.name as api_key_name,
                      ul.channel_id, c.name as channel_name,
                      ul.route_id, ul.requested_model, ul.actual_model,
                      ul.input_tokens, ul.output_tokens,
                      ul.cache_read_tokens, ul.cache_creation_tokens,
                      ul.cost, ul.latency_ms, ul.ttft_ms, ul.status_code, ul.error_message, datetime(ul.created_at, '{}') as created_at,
                      ul.endpoint_type, ul.request_type, ul.is_stream, ul.upstream_key_hint, ul.user_agent, ul.attempts as raw_attempts
               FROM usage_logs ul
               LEFT JOIN api_keys ak ON ul.api_key_id = ak.id
               LEFT JOIN channels c ON ul.channel_id = c.id
               WHERE 1=1"#,
            tz
        ));
        if let Some(ref model) = filter.model {
            data_builder.push(" AND ul.requested_model = ");
            data_builder.push_bind(model.clone());
        }
        if let Some(ref cid) = filter.channel_id {
            data_builder.push(" AND ul.channel_id = ");
            data_builder.push_bind(cid.clone());
        }
        if let Some(ref kid) = filter.api_key_id {
            data_builder.push(" AND ul.api_key_id = ");
            data_builder.push_bind(kid.clone());
        }
        match filter.status.as_deref() {
            Some("success") => {
                data_builder.push(" AND ul.status_code >= 200 AND ul.status_code < 400");
            }
            Some("failure") => {
                data_builder.push(" AND (ul.status_code < 200 OR ul.status_code >= 400)");
            }
            _ => {}
        }
        data_builder.push(" ORDER BY ul.created_at DESC LIMIT ");
        data_builder.push(filter.limit);
        data_builder.push(" OFFSET ");
        data_builder.push(filter.offset);

        let rows: Vec<UsageLogRow> = data_builder
            .build_query_as()
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|mut row: UsageLogRow| {
                row.attempts = row
                    .raw_attempts
                    .take()
                    .and_then(|s| serde_json::from_str(&s).ok());
                row
            })
            .collect();

        Ok(PagedResult {
            items: rows,
            total: total.0,
        })
    }

    /// 获取单条日志详情（含请求/响应内容）
    pub async fn get_log_detail(&self, id: &str) -> Result<Option<UsageLogDetail>, sqlx::Error> {
        let tz = self.tz_modifier();
        let row = sqlx::query_as::<_, UsageLogDetail>(
            AssertSqlSafe(format!(r#"SELECT ul.id, ul.api_key_id, ak.name as api_key_name,
                      ul.channel_id, c.name as channel_name,
                      ul.route_id, ul.requested_model, ul.actual_model,
                      ul.input_tokens, ul.output_tokens,
                      ul.cache_read_tokens, ul.cache_creation_tokens,
                      ul.cost, ul.latency_ms, ul.ttft_ms, ul.status_code, ul.error_message, datetime(ul.created_at, '{}') as created_at,
                      ul.endpoint_type, ul.request_type,
                      ul.request_content, ul.response_content, ul.is_stream, ul.upstream_key_hint, ul.user_agent, ul.attempts as raw_attempts
               FROM usage_logs ul
               LEFT JOIN api_keys ak ON ul.api_key_id = ak.id
               LEFT JOIN channels c ON ul.channel_id = c.id
               WHERE ul.id = ?"#, tz).as_str()),
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .map(|mut r| {
            r.attempts = r.raw_attempts.take().and_then(|s| serde_json::from_str(&s).ok());
            r
        });

        Ok(row)
    }

    /// 获取日志中出现过的不重复模型列表
    pub async fn get_log_models(&self) -> Result<Vec<String>, sqlx::Error> {
        let models = sqlx::query_scalar::<_, String>(
            "SELECT DISTINCT requested_model FROM usage_logs ORDER BY requested_model",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(models)
    }

    /// 按 API Key 聚合统计
    pub async fn get_api_key_stats(&self, days: i32) -> Result<Vec<ApiKeyStats>, sqlx::Error> {
        let cutoff = self.now_local() - chrono::Duration::days(days as i64);
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
        .fetch_all(&self.pool)
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

    /// 将小时级结果补齐到完整 24 小时（00:00 ~ 23:00）
    fn fill_hourly(&self, rows: Vec<DailyRow>) -> Vec<DailyStats> {
        let map: HashMap<String, DailyStats> = rows
            .into_iter()
            .map(|r| {
                let s = daily_row_to_stats(r);
                (s.date.clone(), s)
            })
            .collect();

        let mut result = Vec::with_capacity(24);
        for h in 0..24 {
            let key = format!("{:02}:00", h);
            result.push(map.get(&key).cloned().unwrap_or(DailyStats {
                date: key,
                request_count: 0,
                success_count: 0,
                failure_count: 0,
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                total_cost: 0.0,
            }));
        }
        result
    }
}

fn daily_row_to_stats(row: DailyRow) -> DailyStats {
    DailyStats {
        date: row.0,
        request_count: row.1,
        success_count: row.2,
        failure_count: row.3,
        input_tokens: row.4,
        output_tokens: row.5,
        cache_read_tokens: row.6,
        cache_creation_tokens: row.7,
        total_cost: row.8,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    async fn make_pool() -> sqlx::SqlitePool {
        let db_path = format!("/tmp/galaxy_stats_mod_{}.db", uuid::Uuid::now_v7());
        let _ = std::fs::remove_file(&db_path);
        let db_url = format!("sqlite:{}?mode=rwc", db_path);
        Database::new(&db_url).await.unwrap().pool().clone()
    }

    fn new_state(pool: sqlx::SqlitePool) -> StatsState {
        StatsState::new(pool, 0) // UTC
    }

    async fn seed_log(
        pool: &sqlx::SqlitePool,
        model: &str,
        channel_id: Option<&str>,
        api_key_id: Option<&str>,
        status_code: i32,
        input: i32,
        output: i32,
        cost: f64,
    ) {
        let id = crate::api::response::generate_id();
        sqlx::query(
            r#"INSERT INTO usage_logs
               (id, requested_model, channel_id, api_key_id, status_code,
                input_tokens, output_tokens, cost, request_type, is_stream)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'passthrough', 0)"#,
        )
        .bind(&id)
        .bind(model)
        .bind(channel_id)
        .bind(api_key_id)
        .bind(status_code)
        .bind(input)
        .bind(output)
        .bind(cost)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn get_overview_empty_db() {
        let pool = make_pool().await;
        let state = new_state(pool);
        let ov = state.get_overview().await.unwrap();
        assert_eq!(ov.total_requests, 0);
        assert_eq!(ov.total_input_tokens, 0);
        assert_eq!(ov.total_cost, 0.0);
        assert_eq!(ov.today_requests, 0);
    }

    #[tokio::test]
    async fn get_overview_aggregates_total_and_today() {
        let pool = make_pool().await;
        // 今天的请求
        seed_log(&pool, "gpt-4o", None, None, 200, 100, 50, 0.005).await;
        seed_log(&pool, "gpt-4o", None, None, 200, 200, 100, 0.01).await;
        // 失败的请求
        seed_log(&pool, "gpt-4o", None, None, 500, 50, 0, 0.0).await;
        let state = new_state(pool);
        let ov = state.get_overview().await.unwrap();
        assert_eq!(ov.total_requests, 3);
        assert_eq!(ov.total_input_tokens, 350);
        assert_eq!(ov.total_output_tokens, 150);
        assert!((ov.total_cost - 0.015).abs() < 1e-9);
        assert_eq!(ov.today_requests, 3);
    }

    #[tokio::test]
    async fn get_model_stats_groups_by_model() {
        let pool = make_pool().await;
        seed_log(&pool, "gpt-4o", None, None, 200, 100, 50, 0.001).await;
        seed_log(&pool, "gpt-4o", None, None, 200, 100, 50, 0.001).await;
        seed_log(&pool, "claude", None, None, 200, 200, 100, 0.002).await;
        let state = new_state(pool);
        let stats = state.get_model_stats(7).await.unwrap();
        assert_eq!(stats.len(), 2);
        // 按请求数降序
        assert_eq!(stats[0].model, "gpt-4o");
        assert_eq!(stats[0].request_count, 2);
        assert_eq!(stats[1].model, "claude");
    }

    #[tokio::test]
    async fn get_channel_stats_joins_channel_name() {
        let pool = make_pool().await;
        let channel_id = "ch-1";
        sqlx::query(
            "INSERT INTO channels (id, name, api_keys, endpoints) VALUES (?, 'OpenAI', '[]', '[]')",
        )
        .bind(channel_id)
        .execute(&pool)
        .await
        .unwrap();
        seed_log(&pool, "gpt-4o", Some(channel_id), None, 200, 10, 5, 0.0001).await;
        seed_log(&pool, "gpt-4o", Some(channel_id), None, 500, 10, 0, 0.0).await;
        // 渠道已被删除的日志，LEFT JOIN 应保留并标记 unknown
        seed_log(&pool, "gpt-4o", Some("ghost"), None, 200, 5, 5, 0.0).await;

        let state = new_state(pool);
        let stats = state.get_channel_stats(7).await.unwrap();
        assert_eq!(stats.len(), 2);
        let ch1 = stats.iter().find(|s| s.channel_id == channel_id).unwrap();
        assert_eq!(ch1.channel_name, "OpenAI");
        assert_eq!(ch1.request_count, 2);
        assert_eq!(ch1.success_count, 1);
        assert_eq!(ch1.failure_count, 1);
        let ghost = stats.iter().find(|s| s.channel_id == "ghost").unwrap();
        assert_eq!(ghost.channel_name, "unknown");
    }

    #[tokio::test]
    async fn get_daily_stats_one_day_fills_24_hours() {
        let pool = make_pool().await;
        // 两条今天的日志（默认 created_at = now）
        seed_log(&pool, "gpt-4o", None, None, 200, 10, 5, 0.001).await;
        seed_log(&pool, "claude", None, None, 500, 20, 0, 0.0).await;
        let state = new_state(pool);
        let stats = state.get_daily_stats(1).await.unwrap();
        assert_eq!(stats.len(), 24, "应补齐 24 个小时槽位");
        // 至少有一个槽位有数据
        let non_empty: Vec<&DailyStats> = stats.iter().filter(|s| s.request_count > 0).collect();
        assert!(!non_empty.is_empty());
        // 当前小时的 success+failure 应等于总请求数
        let total_success: i32 = stats.iter().map(|s| s.success_count).sum();
        let total_failure: i32 = stats.iter().map(|s| s.failure_count).sum();
        assert_eq!(total_success + total_failure, 2);
    }

    #[tokio::test]
    async fn get_daily_stats_seven_days_aggregates_by_date() {
        let pool = make_pool().await;
        // 直接写一条 5 天前的日志
        let id = crate::api::response::generate_id();
        sqlx::query(
            r#"INSERT INTO usage_logs
               (id, requested_model, status_code, input_tokens, output_tokens,
                cost, request_type, is_stream, created_at)
               VALUES (?, 'gpt-4o', 200, 100, 50, 0.001, 'passthrough', 0,
                       datetime('now', '-5 days'))"#,
        )
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
        let state = new_state(pool);
        let stats = state.get_daily_stats(7).await.unwrap();
        assert!(!stats.is_empty());
        // 不超过 7 天（含今天）
        assert!(stats.len() <= 7);
    }

    #[tokio::test]
    async fn get_daily_stats_by_range() {
        let pool = make_pool().await;
        seed_log(&pool, "gpt-4o", None, None, 200, 10, 5, 0.001).await;
        let state = new_state(pool);
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let stats = state
            .get_daily_stats_by_range(&today, &today)
            .await
            .unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].date, today);
    }

    #[tokio::test]
    async fn get_model_stats_by_range() {
        let pool = make_pool().await;
        seed_log(&pool, "gpt-4o", None, None, 200, 10, 5, 0.001).await;
        seed_log(&pool, "claude", None, None, 200, 20, 10, 0.002).await;
        let state = new_state(pool);
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let stats = state
            .get_model_stats_by_range(&today, &today)
            .await
            .unwrap();
        assert_eq!(stats.len(), 2);
    }

    #[tokio::test]
    async fn get_channel_stats_by_range() {
        let pool = make_pool().await;
        let cid = "ch-1";
        sqlx::query(
            "INSERT INTO channels (id, name, api_keys, endpoints) VALUES (?, 'A', '[]', '[]')",
        )
        .bind(cid)
        .execute(&pool)
        .await
        .unwrap();
        seed_log(&pool, "gpt-4o", Some(cid), None, 200, 10, 5, 0.001).await;
        let state = new_state(pool);
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let stats = state
            .get_channel_stats_by_range(&today, &today)
            .await
            .unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].channel_name, "A");
    }

    #[tokio::test]
    async fn get_logs_paginated_and_parsed_attempts() {
        let pool = make_pool().await;
        // 写 attempts 字段以验证 JSON 解析路径（生产 SQL 用 `as raw_attempts` 重命名）
        let id = crate::api::response::generate_id();
        let attempts = r#"[{"channel_id":"c1","status":"ok","duration_ms":12}]"#;
        sqlx::query(
            r#"INSERT INTO usage_logs
               (id, requested_model, status_code, input_tokens, output_tokens,
                cost, request_type, is_stream, attempts)
               VALUES (?, 'gpt-4o', 200, 10, 5, 0.001, 'passthrough', 0, ?)"#,
        )
        .bind(&id)
        .bind(attempts)
        .execute(&pool)
        .await
        .unwrap();

        let state = new_state(pool);
        let page = state
            .get_logs(LogsFilter {
                offset: 0,
                limit: 10,
                model: None,
                channel_id: None,
                status: None,
                api_key_id: None,
            })
            .await
            .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items.len(), 1);
        let row = &page.items[0];
        assert!(row.attempts.is_some(), "attempts 应被反序列化为 JSON Value");
    }

    #[tokio::test]
    async fn get_logs_filter_by_model_channel_status() {
        let pool = make_pool().await;
        let cid_a = "ch-a";
        let cid_b = "ch-b";
        sqlx::query(
            "INSERT INTO channels (id, name, api_keys, endpoints) VALUES (?, 'A', '[]', '[]')",
        )
        .bind(cid_a)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO channels (id, name, api_keys, endpoints) VALUES (?, 'B', '[]', '[]')",
        )
        .bind(cid_b)
        .execute(&pool)
        .await
        .unwrap();

        seed_log(&pool, "gpt-4o", Some(cid_a), None, 200, 1, 1, 0.0).await;
        seed_log(&pool, "claude", Some(cid_b), None, 500, 1, 0, 0.0).await;
        let state = new_state(pool);

        // 按 model
        let p = state
            .get_logs(LogsFilter {
                offset: 0,
                limit: 10,
                model: Some("claude".into()),
                channel_id: None,
                status: None,
                api_key_id: None,
            })
            .await
            .unwrap();
        assert_eq!(p.total, 1);
        assert_eq!(p.items[0].requested_model, "claude");

        // 按 status=success（200~399）
        let p = state
            .get_logs(LogsFilter {
                offset: 0,
                limit: 10,
                model: None,
                channel_id: None,
                status: Some("success".into()),
                api_key_id: None,
            })
            .await
            .unwrap();
        assert_eq!(p.total, 1);

        // 按 status=failure
        let p = state
            .get_logs(LogsFilter {
                offset: 0,
                limit: 10,
                model: None,
                channel_id: None,
                status: Some("failure".into()),
                api_key_id: None,
            })
            .await
            .unwrap();
        assert_eq!(p.total, 1);

        // 按 channel
        let p = state
            .get_logs(LogsFilter {
                offset: 0,
                limit: 10,
                model: None,
                channel_id: Some(cid_b.into()),
                status: None,
                api_key_id: None,
            })
            .await
            .unwrap();
        assert_eq!(p.total, 1);
    }

    #[tokio::test]
    async fn get_log_detail_returns_full_row() {
        let pool = make_pool().await;
        let id = crate::api::response::generate_id();
        let attempts = r#"[{"channel_id":"c1","status":"ok","duration_ms":12}]"#;
        sqlx::query(
            r#"INSERT INTO usage_logs
               (id, requested_model, status_code, input_tokens, output_tokens,
                cost, request_type, is_stream, request_content, response_content, attempts)
               VALUES (?, 'gpt-4o', 200, 10, 5, 0.001, 'passthrough', 0, '{}', '{}', ?)"#,
        )
        .bind(&id)
        .bind(attempts)
        .execute(&pool)
        .await
        .unwrap();

        let state = new_state(pool);
        let detail = state
            .get_log_detail(&id)
            .await
            .unwrap()
            .expect("应能取到详情");
        assert_eq!(detail.id, id);
        assert!(detail.request_content.is_some());
        assert!(detail.attempts.is_some());
    }

    #[tokio::test]
    async fn get_log_detail_missing_returns_none() {
        let pool = make_pool().await;
        let state = new_state(pool);
        let detail = state.get_log_detail("nonexistent").await.unwrap();
        assert!(detail.is_none());
    }

    #[tokio::test]
    async fn get_log_models_returns_distinct_sorted() {
        let pool = make_pool().await;
        seed_log(&pool, "gpt-4o", None, None, 200, 1, 1, 0.0).await;
        seed_log(&pool, "claude", None, None, 200, 1, 1, 0.0).await;
        seed_log(&pool, "gpt-4o", None, None, 200, 1, 1, 0.0).await; // 重复
        let state = new_state(pool);
        let models = state.get_log_models().await.unwrap();
        assert_eq!(models, vec!["claude".to_string(), "gpt-4o".to_string()]);
    }

    #[tokio::test]
    async fn get_latency_percentiles_returns_none_on_empty() {
        let pool = make_pool().await;
        let state = new_state(pool);
        let (p50, p95, p99) = state.get_latency_percentiles(7).await.unwrap();
        assert!(p50.is_none());
        assert!(p95.is_none());
        assert!(p99.is_none());
    }

    #[tokio::test]
    async fn get_latency_percentiles_computes_correctly() {
        let pool = make_pool().await;
        // 插入 10 条不同延迟的日志
        for i in 1..=10 {
            let id = crate::api::response::generate_id();
            sqlx::query(
                r#"INSERT INTO usage_logs
                   (id, requested_model, status_code, input_tokens, output_tokens,
                    cost, request_type, is_stream, latency_ms)
                   VALUES (?, 'gpt-4o', 200, 1, 1, 0.0, 'passthrough', 0, ?)"#,
            )
            .bind(&id)
            .bind(i * 100) // 100, 200, ..., 1000
            .execute(&pool)
            .await
            .unwrap();
        }
        let state = new_state(pool);
        let (p50, p95, p99) = state.get_latency_percentiles(7).await.unwrap();
        assert!(p50.is_some());
        assert!(p95.is_some());
        assert!(p99.is_some());
        // 10 个值: 100,200,...,1000. p50 ≈ 550, p95 ≈ 955, p99 ≈ 1000
        assert!(p50.unwrap() > 0.0);
        assert!(p95.unwrap() >= p50.unwrap());
        assert!(p99.unwrap() >= p95.unwrap());
    }
}
