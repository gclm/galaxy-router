//! Usage 数据访问层（按查询类型拆子模块）。
//!
//! 本 mod.rs 持 `UsageRepository` trait + `SqliteUsageRepository` struct + 跨类共享的
//! helper/类型；各查询类别在 leaf 文件（overview/trend/model/channel/api_key/logs/
//! latency/write/maintenance）各自的 `impl UsageRepository for SqliteUsageRepository`
//! 块中实现。行类型（UsageLogRow/UsageLogDetail）带 `sqlx::FromRow` 留本层；纯 DTO 在 domain::usage。

use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::domain::usage::{
    ApiKeyStats, ChannelStats, DailyStats, LogsFilter, ModelStats, PagedResult, RequestRecord,
    StatsOverview,
};
use crate::util::timeutil::tz_modifier;

mod api_key;
mod channel;
mod latency;
mod logs;
mod maintenance;
mod model;
mod overview;
mod trend;
mod write;

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

/// 按日期聚合的统计行
pub(super) type DailyRow = (String, i32, i32, i32, i32, i32, i32, i32, f64);

#[async_trait]
pub trait UsageRepository: Send + Sync {
    async fn get_overview(&self) -> Result<StatsOverview, sqlx::Error>;
    /// 计算延迟百分位（p50/p95/p99）
    async fn get_latency_percentiles(
        &self,
        days: i32,
    ) -> Result<(Option<f64>, Option<f64>, Option<f64>), sqlx::Error>;
    async fn get_model_stats(&self, days: i32) -> Result<Vec<ModelStats>, sqlx::Error>;
    async fn get_channel_stats(&self, days: i32) -> Result<Vec<ChannelStats>, sqlx::Error>;
    async fn get_daily_stats(&self, days: i32) -> Result<Vec<DailyStats>, sqlx::Error>;
    async fn get_daily_stats_by_range(
        &self,
        start: &str,
        end: &str,
    ) -> Result<Vec<DailyStats>, sqlx::Error>;
    async fn get_model_stats_by_range(
        &self,
        start: &str,
        end: &str,
    ) -> Result<Vec<ModelStats>, sqlx::Error>;
    async fn get_channel_stats_by_range(
        &self,
        start: &str,
        end: &str,
    ) -> Result<Vec<ChannelStats>, sqlx::Error>;
    async fn get_logs(&self, filter: LogsFilter) -> Result<PagedResult<UsageLogRow>, sqlx::Error>;
    async fn get_log_detail(&self, id: &str) -> Result<Option<UsageLogDetail>, sqlx::Error>;
    async fn get_log_models(&self) -> Result<Vec<String>, sqlx::Error>;
    async fn get_api_key_stats(&self, days: i32) -> Result<Vec<ApiKeyStats>, sqlx::Error>;

    /// 写入 usage_logs（统计字段），返回生成的 log_id（UUID v7）。B2-C1 落库下沉。
    async fn insert_usage_log(&self, record: &RequestRecord) -> Result<String, sqlx::Error>;

    /// 写入 usage_payloads（已脱敏的请求/响应原文）。B2-C1 落库下沉。
    async fn insert_usage_payload(
        &self,
        log_id: &str,
        request_content: Option<&str>,
        response_content: Option<&str>,
    ) -> Result<(), sqlx::Error>;

    /// proxy 预算门：按 api_key_id 取 (月消费, 日消费)，均按 UTC（与 created_at 存储一致）。
    async fn aggregate_cost(&self, api_key_id: &str) -> Result<(f64, f64), sqlx::Error>;
    /// 保留清理：删除 N 天前的 usage_logs，返回删除行数。
    async fn delete_older_than(&self, days: i64) -> Result<u64, sqlx::Error>;
}

pub struct SqliteUsageRepository {
    pool: SqlitePool,
    timezone_offset: i32,
}

impl SqliteUsageRepository {
    pub fn new(pool: SqlitePool, timezone_offset: i32) -> Self {
        Self {
            pool,
            timezone_offset,
        }
    }

    pub(super) fn tz_modifier(&self) -> String {
        tz_modifier(self.timezone_offset)
    }

    pub(super) fn now_local(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now() + chrono::Duration::hours(self.timezone_offset as i64)
    }

    /// 本地"最近 days 天(含今天)"对应的 UTC 时间范围 [start, end)。
    /// created_at 以 UTC 存储,用裸列 `created_at >= ? AND created_at < ?` 比较才能命中
    /// idx_usage_logs_created_at(避免 `date(datetime(created_at,...))` 让索引失效、全表扫)。
    pub(super) fn range_utc_days(&self, days: i32) -> (String, String) {
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
    pub(super) fn range_utc_between(&self, start_local: &str, end_local: &str) -> (String, String) {
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

    /// 将小时级结果补齐到完整 24 小时（00:00 ~ 23:00）
    pub(super) fn fill_hourly(&self, rows: Vec<DailyRow>) -> Vec<DailyStats> {
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

pub(super) fn daily_row_to_stats(row: DailyRow) -> DailyStats {
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

/// 完整 trait 实现：每方法委托到对应查询类别的 leaf 模块（实际 SQL 在各 leaf）。
/// trait impl 不可拆多块（Rust 约束），故集中在此作薄委托。
#[async_trait]
impl UsageRepository for SqliteUsageRepository {
    async fn get_overview(&self) -> Result<StatsOverview, sqlx::Error> {
        overview::get_overview(self).await
    }
    async fn get_latency_percentiles(
        &self,
        days: i32,
    ) -> Result<(Option<f64>, Option<f64>, Option<f64>), sqlx::Error> {
        latency::get_latency_percentiles(self, days).await
    }
    async fn get_model_stats(&self, days: i32) -> Result<Vec<ModelStats>, sqlx::Error> {
        model::get_model_stats(self, days).await
    }
    async fn get_channel_stats(&self, days: i32) -> Result<Vec<ChannelStats>, sqlx::Error> {
        channel::get_channel_stats(self, days).await
    }
    async fn get_daily_stats(&self, days: i32) -> Result<Vec<DailyStats>, sqlx::Error> {
        trend::get_daily_stats(self, days).await
    }
    async fn get_daily_stats_by_range(
        &self,
        start: &str,
        end: &str,
    ) -> Result<Vec<DailyStats>, sqlx::Error> {
        trend::get_daily_stats_by_range(self, start, end).await
    }
    async fn get_model_stats_by_range(
        &self,
        start: &str,
        end: &str,
    ) -> Result<Vec<ModelStats>, sqlx::Error> {
        model::get_model_stats_by_range(self, start, end).await
    }
    async fn get_channel_stats_by_range(
        &self,
        start: &str,
        end: &str,
    ) -> Result<Vec<ChannelStats>, sqlx::Error> {
        channel::get_channel_stats_by_range(self, start, end).await
    }
    async fn get_logs(&self, filter: LogsFilter) -> Result<PagedResult<UsageLogRow>, sqlx::Error> {
        logs::get_logs(self, filter).await
    }
    async fn get_log_detail(&self, id: &str) -> Result<Option<UsageLogDetail>, sqlx::Error> {
        logs::get_log_detail(self, id).await
    }
    async fn get_log_models(&self) -> Result<Vec<String>, sqlx::Error> {
        logs::get_log_models(self).await
    }
    async fn get_api_key_stats(&self, days: i32) -> Result<Vec<ApiKeyStats>, sqlx::Error> {
        api_key::get_api_key_stats(self, days).await
    }
    async fn insert_usage_log(&self, record: &RequestRecord) -> Result<String, sqlx::Error> {
        write::insert_usage_log(self, record).await
    }
    async fn insert_usage_payload(
        &self,
        log_id: &str,
        request_content: Option<&str>,
        response_content: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        write::insert_usage_payload(self, log_id, request_content, response_content).await
    }
    async fn aggregate_cost(&self, api_key_id: &str) -> Result<(f64, f64), sqlx::Error> {
        maintenance::aggregate_cost(self, api_key_id).await
    }
    async fn delete_older_than(&self, days: i64) -> Result<u64, sqlx::Error> {
        maintenance::delete_older_than(self, days).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::usage::{DailyStats, LogsFilter};
    use crate::infra::db::Database;

    async fn make_pool() -> sqlx::SqlitePool {
        let db_path = format!("/tmp/galaxy_stats_mod_{}.db", uuid::Uuid::now_v7());
        let _ = std::fs::remove_file(&db_path);
        let db_url = format!("sqlite:{}?mode=rwc", db_path);
        Database::new(&db_url).await.unwrap().pool().clone()
    }

    fn new_state(pool: sqlx::SqlitePool) -> SqliteUsageRepository {
        SqliteUsageRepository::new(pool, 0) // UTC
    }

    #[allow(clippy::too_many_arguments)]
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
        let id = crate::util::id::generate_id();
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
        let id = crate::util::id::generate_id();
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
        let id = crate::util::id::generate_id();
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
        let id = crate::util::id::generate_id();
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
        // content 拆到 usage_payloads（Step E 拆表）
        sqlx::query(
            "INSERT INTO usage_payloads (log_id, request_content, response_content) VALUES (?, '{}', '{}')",
        )
        .bind(&id)
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
            let id = crate::util::id::generate_id();
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
