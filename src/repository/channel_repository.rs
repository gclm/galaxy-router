//! Channel 数据访问层（v1.1.2 从 channels/crud.rs 迁入）。
//!
//! SQL 隔离在此层，返回 `ChannelRow`（FromRow，B1-C4 归位本层）由 handler 调
//! `row_to_channel`（B1-C4 同归位）转 `Channel`。
//!
//! 请求 DTO（Create/Update/List）已归 `domain::channel`（A1 解耦），本层不再借 api 层类型。

use async_trait::async_trait;
use sqlx::{AssertSqlSafe, Row, SqlitePool};

use crate::domain::channel::{
    Channel, CreateChannelRequest, ListChannelsQuery, UpdateChannelRequest,
};
use crate::util::timeutil::tz_modifier;

/// channels 表行映射（B1-C4 从 channels/types.rs 归位本层）
#[derive(sqlx::FromRow)]
pub struct ChannelRow {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) api_keys: String,
    pub(crate) endpoints: String,
    pub(crate) models: String,
    pub(crate) rate_limit_rpm: Option<i32>,
    pub(crate) rate_limit_tpm: Option<i32>,
    pub(crate) failure_threshold: i32,
    pub(crate) blacklist_minutes: i32,
    pub(crate) concurrency: i32,
    pub(crate) timeout_secs: i32,
    pub(crate) max_concurrency: i32,
    pub(crate) enabled: bool,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

#[async_trait]
pub trait ChannelRepository: Send + Sync {
    async fn list(&self, query: ListChannelsQuery) -> Result<(Vec<ChannelRow>, i64), sqlx::Error>;
    async fn create(&self, req: CreateChannelRequest) -> Result<ChannelRow, sqlx::Error>;
    async fn get(&self, id: &str) -> Result<Option<ChannelRow>, sqlx::Error>;
    /// 更新。返回 None=不存在
    async fn update(&self, id: &str, req: UpdateChannelRequest) -> Result<Option<ChannelRow>, sqlx::Error>;
    /// 删除（事务：route_items + channel）。返回 None=不存在；Some=受影响 route_ids（供 handler 缓存失效）
    async fn delete(&self, id: &str) -> Result<Option<Vec<String>>, sqlx::Error>;
}

pub struct SqliteChannelRepository {
    pool: SqlitePool,
    timezone_offset: i32,
}

impl SqliteChannelRepository {
    pub fn new(pool: SqlitePool, timezone_offset: i32) -> Self {
        Self {
            pool,
            timezone_offset,
        }
    }

    fn tz_modifier(&self) -> String {
        tz_modifier(self.timezone_offset)
    }

    /// 带 tz 的 channels 列清单（list/get 共用）
    fn select_columns(&self) -> String {
        let tz = self.tz_modifier();
        format!(
            "id, name, api_keys, endpoints, models, rate_limit_rpm, rate_limit_tpm, failure_threshold, blacklist_minutes, concurrency, timeout_secs, max_concurrency, enabled, datetime(created_at, '{}') as created_at, datetime(updated_at, '{}') as updated_at",
            tz, tz
        )
    }

    fn push_where(builder: &mut sqlx::QueryBuilder<sqlx::Sqlite>, query: &ListChannelsQuery) -> bool {
        let mut has_where = false;

        if let Some(ref search) = query.search
            && !search.is_empty()
        {
            builder.push(" WHERE name LIKE ");
            builder.push_bind(format!("%{}%", search));
            has_where = true;
        }
        if let Some(ref status) = query.status {
            let enabled_val = match status.as_str() {
                "enabled" => Some(true),
                "disabled" => Some(false),
                _ => None,
            };
            if let Some(v) = enabled_val {
                builder.push(if has_where {
                    " AND enabled = "
                } else {
                    " WHERE enabled = "
                });
                builder.push_bind(v);
                has_where = true;
            }
        }
        has_where
    }
}

#[async_trait]
impl ChannelRepository for SqliteChannelRepository {
    async fn list(&self, query: ListChannelsQuery) -> Result<(Vec<ChannelRow>, i64), sqlx::Error> {
        let page = query.page.unwrap_or(1).max(1);
        let page_size = query.page_size.unwrap_or(20).clamp(1, 100);
        let offset = (page - 1) * page_size;

        let order_field = match query.sort_by.as_deref() {
            Some("name") => "name",
            _ => "created_at",
        };
        let order_dir = match query.sort_order.as_deref() {
            Some("asc") => "ASC",
            _ => "DESC",
        };

        let mut count_builder = sqlx::QueryBuilder::new("SELECT COUNT(*) FROM channels");
        Self::push_where(&mut count_builder, &query);
        let count_row = count_builder.build().fetch_one(&self.pool).await?;
        let total: i64 = count_row.get(0);

        let cols = self.select_columns();
        let mut data_builder = sqlx::QueryBuilder::new(format!("SELECT {} FROM channels", cols));
        Self::push_where(&mut data_builder, &query);
        data_builder.push(format!(" ORDER BY {} {} ", order_field, order_dir));
        data_builder.push(" LIMIT ");
        data_builder.push_bind(page_size);
        data_builder.push(" OFFSET ");
        data_builder.push_bind(offset);

        let rows: Vec<ChannelRow> = data_builder.build_query_as().fetch_all(&self.pool).await?;

        Ok((rows, total))
    }

    async fn create(&self, req: CreateChannelRequest) -> Result<ChannelRow, sqlx::Error> {
        let id = crate::util::id::generate_id();
        let api_keys_json = serde_json::to_string(&req.api_keys).unwrap_or_default();
        let endpoints_json = serde_json::to_string(&req.endpoints).unwrap_or_default();
        let models_json = serde_json::to_string(&req.models.unwrap_or_default()).unwrap_or_default();

        sqlx::query(
            r#"INSERT INTO channels (id, name, api_keys, endpoints, models, rate_limit_rpm, rate_limit_tpm, failure_threshold, blacklist_minutes, concurrency, timeout_secs, max_concurrency, enabled)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(&id)
        .bind(&req.name)
        .bind(&api_keys_json)
        .bind(&endpoints_json)
        .bind(&models_json)
        .bind(req.rate_limit_rpm)
        .bind(req.rate_limit_tpm)
        .bind(req.failure_threshold.unwrap_or(3))
        .bind(req.blacklist_minutes.unwrap_or(10))
        .bind(req.concurrency.unwrap_or(10))
        .bind(req.timeout_secs.unwrap_or(300))
        .bind(req.max_concurrency.unwrap_or(0))
        .bind(req.enabled.unwrap_or(true))
        .execute(&self.pool)
        .await?;

        self.get(&id).await?.ok_or(sqlx::Error::RowNotFound)
    }

    async fn get(&self, id: &str) -> Result<Option<ChannelRow>, sqlx::Error> {
        let cols = self.select_columns();
        let row = sqlx::query_as::<_, ChannelRow>(
            AssertSqlSafe(format!("SELECT {} FROM channels WHERE id = ?", cols)),
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn update(
        &self,
        id: &str,
        req: UpdateChannelRequest,
    ) -> Result<Option<ChannelRow>, sqlx::Error> {
        let existing = sqlx::query_scalar::<_, String>("SELECT id FROM channels WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        if existing.is_none() {
            return Ok(None);
        }

        let mut builder = sqlx::QueryBuilder::new("UPDATE channels SET ");
        let mut separated = builder.separated(", ");
        if let Some(ref name) = req.name {
            separated.push("name = ");
            separated.push_bind_unseparated(name);
        }
        if let Some(ref api_keys) = req.api_keys {
            separated.push("api_keys = ");
            separated.push_bind_unseparated(serde_json::to_string(api_keys).unwrap_or_default());
        }
        if let Some(ref endpoints) = req.endpoints {
            separated.push("endpoints = ");
            separated.push_bind_unseparated(serde_json::to_string(endpoints).unwrap_or_default());
        }
        if let Some(ref models) = req.models {
            separated.push("models = ");
            separated.push_bind_unseparated(serde_json::to_string(models).unwrap_or_default());
        }
        if let Some(enabled) = req.enabled {
            separated.push("enabled = ");
            separated.push_bind_unseparated(enabled);
        }
        if let Some(rate_limit_rpm) = req.rate_limit_rpm {
            separated.push("rate_limit_rpm = ");
            separated.push_bind_unseparated(rate_limit_rpm);
        }
        if let Some(rate_limit_tpm) = req.rate_limit_tpm {
            separated.push("rate_limit_tpm = ");
            separated.push_bind_unseparated(rate_limit_tpm);
        }
        if let Some(failure_threshold) = req.failure_threshold {
            separated.push("failure_threshold = ");
            separated.push_bind_unseparated(failure_threshold);
        }
        if let Some(blacklist_minutes) = req.blacklist_minutes {
            separated.push("blacklist_minutes = ");
            separated.push_bind_unseparated(blacklist_minutes);
        }
        if let Some(concurrency) = req.concurrency {
            separated.push("concurrency = ");
            separated.push_bind_unseparated(concurrency);
        }
        if let Some(timeout_secs) = req.timeout_secs {
            separated.push("timeout_secs = ");
            separated.push_bind_unseparated(timeout_secs);
        }
        if let Some(max_concurrency) = req.max_concurrency {
            separated.push("max_concurrency = ");
            separated.push_bind_unseparated(max_concurrency);
        }
        separated.push("updated_at = CURRENT_TIMESTAMP");

        builder.push(" WHERE id = ");
        builder.push_bind(id);
        builder.build().execute(&self.pool).await?;

        self.get(id).await
    }

    async fn delete(&self, id: &str) -> Result<Option<Vec<String>>, sqlx::Error> {
        let mut tx = self.pool.begin().await?;

        // 查询引用该渠道的分组 ID（用于后续清除缓存）
        let affected_route_ids: Vec<String> =
            sqlx::query_scalar("SELECT DISTINCT route_id FROM route_items WHERE channel_id = ?")
                .bind(id)
                .fetch_all(&mut *tx)
                .await?;

        sqlx::query("DELETE FROM route_items WHERE channel_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;

        let result = sqlx::query("DELETE FROM channels WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        tx.commit().await?;
        Ok(Some(affected_route_ids))
    }
}

/// JSON 字段反序列化（row_to_channel 内部用，B1-C4 从 channels/crud.rs 归位）
fn decode_json_field<T>(field_name: &str, value: &str) -> Result<T, String>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_str(value).map_err(|e| format!("解析 {} 失败: {}", field_name, e))
}

/// ChannelRow → Channel 领域转换（B1-C4 从 channels/crud.rs 归位本层）
pub(crate) fn row_to_channel(row: ChannelRow) -> Result<Channel, String> {
    Ok(Channel {
        id: row.id,
        name: row.name,
        api_keys: decode_json_field("channels.api_keys", &row.api_keys)?,
        endpoints: decode_json_field("channels.endpoints", &row.endpoints)?,
        models: decode_json_field("channels.models", &row.models)?,
        rate_limit_rpm: row.rate_limit_rpm,
        rate_limit_tpm: row.rate_limit_tpm,
        failure_threshold: row.failure_threshold,
        blacklist_minutes: row.blacklist_minutes,
        concurrency: row.concurrency,
        timeout_secs: row.timeout_secs,
        max_concurrency: row.max_concurrency,
        enabled: row.enabled,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}
