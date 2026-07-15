//! ApiKey 数据访问层（v1.1.2 从 api_keys.rs 迁入）。
//!
//! `update`/`delete` 返回值携带 `api_key` 字段值，供 handler 失效 ApiKeyCache（副作用留 handler）。

use async_trait::async_trait;
use sqlx::{AssertSqlSafe, Row, SqlitePool};

use crate::domain::api_key::{
    ApiKey, CreateApiKeyRequest, ListApiKeysQuery, UpdateApiKeyRequest,
};
use crate::util::timeutil::{now_local_str, tz_modifier};

#[async_trait]
pub trait ApiKeyRepository: Send + Sync {
    async fn list(&self, query: ListApiKeysQuery) -> Result<(Vec<ApiKey>, i64), sqlx::Error>;
    async fn create(&self, req: CreateApiKeyRequest) -> Result<ApiKey, sqlx::Error>;
    async fn get(&self, id: &str) -> Result<Option<ApiKey>, sqlx::Error>;
    /// 更新。返回 None=不存在；返回的 ApiKey 含 api_key 值供 handler 失效缓存
    async fn update(&self, id: &str, req: UpdateApiKeyRequest) -> Result<Option<ApiKey>, sqlx::Error>;
    /// 删除。返回 None=不存在；Some=被删的 api_key 值供 handler 失效缓存
    async fn delete(&self, id: &str) -> Result<Option<String>, sqlx::Error>;

    /// proxy 鉴权：按 api_key 值查 (id, name, enabled, rate_limit_rpm, rate_limit_tpm, allowed_routes)。
    async fn find_for_auth(
        &self,
        api_key: &str,
    ) -> Result<Option<(String, String, bool, i32, i32, String)>, sqlx::Error>;
    /// proxy 模型访问：按 id 取 supported_models 原始字符串。
    async fn get_supported_models(&self, id: &str) -> Result<Option<String>, sqlx::Error>;
}

pub struct SqliteApiKeyRepository {
    pool: SqlitePool,
    timezone_offset: i32,
}

impl SqliteApiKeyRepository {
    pub fn new(pool: SqlitePool, timezone_offset: i32) -> Self {
        Self {
            pool,
            timezone_offset,
        }
    }

    fn tz_modifier(&self) -> String {
        tz_modifier(self.timezone_offset)
    }

    fn push_where(builder: &mut sqlx::QueryBuilder<sqlx::Sqlite>, query: &ListApiKeysQuery) -> bool {
        let mut has_where = false;

        if let Some(ref search) = query.search
            && !search.is_empty()
        {
            builder.push(" WHERE name LIKE ");
            builder.push_bind(format!("%{search}%"));
            has_where = true;
        }

        if let Some(ref status) = query.status {
            if has_where {
                builder.push(" AND enabled = ");
            } else {
                builder.push(" WHERE enabled = ");
            }
            builder.push_bind(status == "enabled");
            has_where = true;
        }

        has_where
    }
}

#[async_trait]
impl ApiKeyRepository for SqliteApiKeyRepository {
    async fn list(&self, query: ListApiKeysQuery) -> Result<(Vec<ApiKey>, i64), sqlx::Error> {
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

        let mut count_builder = sqlx::QueryBuilder::new("SELECT COUNT(*) FROM api_keys");
        Self::push_where(&mut count_builder, &query);
        let count_row = count_builder.build().fetch_one(&self.pool).await?;
        let total: i64 = count_row.get(0);

        let tz = self.tz_modifier();
        let mut data_builder = sqlx::QueryBuilder::new(format!(
            "SELECT id, name, api_key, enabled, COALESCE(supported_models, '') as supported_models, COALESCE(rate_limit_rpm, 0) as rate_limit_rpm, COALESCE(rate_limit_tpm, 0) as rate_limit_tpm, COALESCE(allowed_routes, '') as allowed_routes, datetime(created_at, '{}') as created_at, datetime(updated_at, '{}') as updated_at FROM api_keys",
            tz, tz
        ));
        Self::push_where(&mut data_builder, &query);
        data_builder.push(format!(" ORDER BY {} {} ", order_field, order_dir));
        data_builder.push(" LIMIT ");
        data_builder.push_bind(page_size);
        data_builder.push(" OFFSET ");
        data_builder.push_bind(offset);

        let rows = data_builder.build().fetch_all(&self.pool).await?;

        let items: Vec<ApiKey> = rows
            .iter()
            .map(|row| ApiKey {
                id: row.get("id"),
                name: row.get("name"),
                api_key: row.get("api_key"),
                enabled: row.get("enabled"),
                supported_models: empty_to_none(row.get::<String, _>("supported_models")),
                rate_limit_rpm: row.get("rate_limit_rpm"),
                rate_limit_tpm: row.get("rate_limit_tpm"),
                allowed_routes: empty_to_none(row.get::<String, _>("allowed_routes")),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
            })
            .collect();

        Ok((items, total))
    }

    async fn create(&self, req: CreateApiKeyRequest) -> Result<ApiKey, sqlx::Error> {
        let id = crate::util::id::generate_id();
        let api_key = format!("sk-gr-{}", crate::util::id::generate_id());
        let supported_models = req.supported_models.unwrap_or_default();
        let rate_limit_rpm = req.rate_limit_rpm.unwrap_or(0);
        let rate_limit_tpm = req.rate_limit_tpm.unwrap_or(0);
        let allowed_routes = req.allowed_routes.unwrap_or_default();

        sqlx::query(
            r#"INSERT INTO api_keys (id, name, api_key, enabled, supported_models, rate_limit_rpm, rate_limit_tpm, allowed_routes)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(&id)
        .bind(&req.name)
        .bind(&api_key)
        .bind(true)
        .bind(&supported_models)
        .bind(rate_limit_rpm)
        .bind(rate_limit_tpm)
        .bind(&allowed_routes)
        .execute(&self.pool)
        .await?;

        Ok(ApiKey {
            id,
            name: req.name,
            api_key,
            enabled: true,
            supported_models: if supported_models.is_empty() {
                None
            } else {
                Some(supported_models)
            },
            rate_limit_rpm,
            rate_limit_tpm,
            allowed_routes: if allowed_routes.is_empty() {
                None
            } else {
                Some(allowed_routes)
            },
            created_at: now_local_str(self.timezone_offset),
            updated_at: now_local_str(self.timezone_offset),
        })
    }

    async fn get(&self, id: &str) -> Result<Option<ApiKey>, sqlx::Error> {
        let tz = self.tz_modifier();
        let result = sqlx::query_as::<_, (String, String, String, bool, String, i32, i32, String, String, String)>(
            AssertSqlSafe(format!("SELECT id, name, api_key, enabled, supported_models, COALESCE(rate_limit_rpm, 0), COALESCE(rate_limit_tpm, 0), COALESCE(allowed_routes, ''), datetime(created_at, '{tz}') as created_at, datetime(updated_at, '{tz}') as updated_at FROM api_keys WHERE id = ?").as_str()),
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        let Some((id, name, api_key, enabled, supported_models, rate_limit_rpm, rate_limit_tpm, allowed_routes, created_at, updated_at)) = result else {
            return Ok(None);
        };

        Ok(Some(ApiKey {
            id,
            name,
            api_key,
            enabled,
            supported_models: empty_to_none(supported_models),
            rate_limit_rpm,
            rate_limit_tpm,
            allowed_routes: empty_to_none(allowed_routes),
            created_at,
            updated_at,
        }))
    }

    async fn update(&self, id: &str, req: UpdateApiKeyRequest) -> Result<Option<ApiKey>, sqlx::Error> {
        let existing = sqlx::query_scalar::<_, String>("SELECT api_key FROM api_keys WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        if existing.is_none() {
            return Ok(None);
        }

        let mut builder = sqlx::QueryBuilder::new("UPDATE api_keys SET ");
        let mut separated = builder.separated(", ");
        if let Some(ref name) = req.name {
            separated.push("name = ");
            separated.push_bind_unseparated(name);
        }
        if let Some(enabled) = req.enabled {
            separated.push("enabled = ");
            separated.push_bind_unseparated(enabled);
        }
        if let Some(ref supported_models) = req.supported_models {
            separated.push("supported_models = ");
            separated.push_bind_unseparated(supported_models);
        }
        if let Some(rpm) = req.rate_limit_rpm {
            separated.push("rate_limit_rpm = ");
            separated.push_bind_unseparated(rpm);
        }
        if let Some(tpm) = req.rate_limit_tpm {
            separated.push("rate_limit_tpm = ");
            separated.push_bind_unseparated(tpm);
        }
        if let Some(ref allowed_routes) = req.allowed_routes {
            separated.push("allowed_routes = ");
            separated.push_bind_unseparated(allowed_routes);
        }
        separated.push("updated_at = CURRENT_TIMESTAMP");

        builder.push(" WHERE id = ");
        builder.push_bind(id);
        builder.build().execute(&self.pool).await?;

        self.get(id).await
    }

    async fn delete(&self, id: &str) -> Result<Option<String>, sqlx::Error> {
        let api_key_value = sqlx::query_scalar::<_, String>("SELECT api_key FROM api_keys WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        let Some(api_key_value) = api_key_value else {
            return Ok(None);
        };

        let result = sqlx::query("DELETE FROM api_keys WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        Ok(Some(api_key_value))
    }

    async fn find_for_auth(
        &self,
        api_key: &str,
    ) -> Result<Option<(String, String, bool, i32, i32, String)>, sqlx::Error> {
        sqlx::query_as::<_, (String, String, bool, i32, i32, String)>(
            "SELECT id, name, enabled, COALESCE(rate_limit_rpm, 0), COALESCE(rate_limit_tpm, 0), COALESCE(allowed_routes, '') FROM api_keys WHERE api_key = ?",
        )
        .bind(api_key)
        .fetch_optional(&self.pool)
        .await
    }

    async fn get_supported_models(&self, id: &str) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar::<_, String>("SELECT supported_models FROM api_keys WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }
}

/// 空字符串转 None
fn empty_to_none(s: String) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}
