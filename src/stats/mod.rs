pub mod model;
pub mod pricing_refresher;
pub mod recorder;

use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, SqlitePool};
use std::collections::HashMap;

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
        if self.timezone_offset >= 0 {
            format!("+{} hours", self.timezone_offset)
        } else {
            format!("-{} hours", self.timezone_offset.abs())
        }
    }

    fn now_local(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now() + chrono::Duration::hours(self.timezone_offset as i64)
    }

    fn today_local(&self) -> String {
        self.now_local().format("%Y-%m-%d").to_string()
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
    pub group_id: Option<String>,
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
    pub group_id: Option<String>,
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
        let today = self.today_local();
        let tz = self.tz_modifier();

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

        let today_sql = format!(
            "SELECT
                COUNT(*),
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                CAST(COALESCE(SUM(COALESCE(cost, 0)), 0.0) AS REAL)
            FROM usage_logs
            WHERE date(datetime(created_at, '{}')) = ?",
            tz
        );
        let today_stats: (i64, i64, i64, f64) = sqlx::query_as(AssertSqlSafe(today_sql))
            .bind(&today)
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
        })
    }

    /// 获取按模型统计
    pub async fn get_model_stats(&self, days: i32) -> Result<Vec<ModelStats>, sqlx::Error> {
        let start_date = (self.now_local() - chrono::Duration::days(days as i64))
            .format("%Y-%m-%d")
            .to_string();
        let tz = self.tz_modifier();

        let sql = format!(
            "SELECT
                requested_model,
                COUNT(*),
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                CAST(COALESCE(SUM(COALESCE(cost, 0)), 0.0) AS REAL)
            FROM usage_logs
            WHERE date(datetime(created_at, '{}')) >= ?
            GROUP BY requested_model
            ORDER BY COUNT(*) DESC",
            tz
        );
        let stats = sqlx::query_as::<_, (String, i32, i32, i32, f64)>(AssertSqlSafe(sql))
            .bind(&start_date)
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
        let start_date = (self.now_local() - chrono::Duration::days(days as i64))
            .format("%Y-%m-%d")
            .to_string();
        let tz = self.tz_modifier();

        let sql = format!(
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
            WHERE date(datetime(ul.created_at, '{}')) >= ?
            GROUP BY ul.channel_id
            ORDER BY COUNT(*) DESC",
            tz
        );
        let rows: Vec<ChannelStatsRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(&start_date)
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
            let today = self.today_local();
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
                WHERE date(datetime(created_at, '{}')) = ?
                GROUP BY strftime('%H', datetime(created_at, '{}'))
                ORDER BY strftime('%H', datetime(created_at, '{}')) ASC",
                tz, tz, tz, tz
            );
            let rows: Vec<DailyRow> = sqlx::query_as(AssertSqlSafe(sql))
                .bind(&today)
                .fetch_all(&self.pool)
                .await?;

            return Ok(self.fill_hourly(rows));
        }

        let start_date = (self.now_local() - chrono::Duration::days(days as i64))
            .format("%Y-%m-%d")
            .to_string();

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
            WHERE date(datetime(created_at, '{}')) >= ?
            GROUP BY date(datetime(created_at, '{}'))
            ORDER BY date(datetime(created_at, '{}')) ASC",
            tz, tz, tz, tz
        );
        let rows: Vec<DailyRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(&start_date)
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
            WHERE date(datetime(created_at, '{}')) >= ? AND date(datetime(created_at, '{}')) <= ?
            GROUP BY date(datetime(created_at, '{}'))
            ORDER BY date(datetime(created_at, '{}')) ASC",
            tz, tz, tz, tz, tz
        );
        let rows: Vec<DailyRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(start)
            .bind(end)
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
        let tz = self.tz_modifier();
        let sql = format!(
            "SELECT
                requested_model,
                COUNT(*),
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                CAST(COALESCE(SUM(COALESCE(cost, 0)), 0.0) AS REAL)
            FROM usage_logs
            WHERE date(datetime(created_at, '{}')) >= ? AND date(datetime(created_at, '{}')) <= ?
            GROUP BY requested_model
            ORDER BY COUNT(*) DESC",
            tz, tz
        );
        let stats = sqlx::query_as::<_, (String, i32, i32, i32, f64)>(AssertSqlSafe(sql))
            .bind(start)
            .bind(end)
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
        let tz = self.tz_modifier();
        let sql = format!(
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
            WHERE date(datetime(ul.created_at, '{}')) >= ? AND date(datetime(ul.created_at, '{}')) <= ?
            GROUP BY ul.channel_id
            ORDER BY COUNT(*) DESC", tz, tz
        );
        let rows: Vec<ChannelStatsRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(start)
            .bind(end)
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
            count_builder.push(model.clone());
        }
        if let Some(ref cid) = filter.channel_id {
            count_builder.push(" AND ul.channel_id = ");
            count_builder.push(cid.clone());
        }
        if let Some(ref kid) = filter.api_key_id {
            count_builder.push(" AND ul.api_key_id = ");
            count_builder.push(kid.clone());
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

        let mut data_builder = QueryBuilder::new(
            r#"SELECT ul.id, ul.api_key_id, ak.name as api_key_name,
                      ul.channel_id, c.name as channel_name,
                      ul.group_id, ul.requested_model, ul.actual_model,
                      ul.input_tokens, ul.output_tokens,
                      ul.cache_read_tokens, ul.cache_creation_tokens,
                      ul.cost, ul.latency_ms, ul.ttft_ms, ul.status_code, ul.error_message, ul.created_at,
                      ul.endpoint_type, ul.request_type, ul.is_stream, ul.upstream_key_hint, ul.user_agent, ul.attempts as raw_attempts
               FROM usage_logs ul
               LEFT JOIN api_keys ak ON ul.api_key_id = ak.id
               LEFT JOIN channels c ON ul.channel_id = c.id
               WHERE 1=1"#,
        );
        if let Some(ref model) = filter.model {
            data_builder.push(" AND ul.requested_model = ");
            data_builder.push(model.clone());
        }
        if let Some(ref cid) = filter.channel_id {
            data_builder.push(" AND ul.channel_id = ");
            data_builder.push(cid.clone());
        }
        if let Some(ref kid) = filter.api_key_id {
            data_builder.push(" AND ul.api_key_id = ");
            data_builder.push(kid.clone());
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
        let row = sqlx::query_as::<_, UsageLogDetail>(
            r#"SELECT ul.id, ul.api_key_id, ak.name as api_key_name,
                      ul.channel_id, c.name as channel_name,
                      ul.group_id, ul.requested_model, ul.actual_model,
                      ul.input_tokens, ul.output_tokens,
                      ul.cache_read_tokens, ul.cache_creation_tokens,
                      ul.cost, ul.latency_ms, ul.ttft_ms, ul.status_code, ul.error_message, ul.created_at,
                      ul.endpoint_type, ul.request_type,
                      ul.request_content, ul.response_content, ul.is_stream, ul.upstream_key_hint, ul.user_agent, ul.attempts as raw_attempts
               FROM usage_logs ul
               LEFT JOIN api_keys ak ON ul.api_key_id = ak.id
               LEFT JOIN channels c ON ul.channel_id = c.id
               WHERE ul.id = ?"#,
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
