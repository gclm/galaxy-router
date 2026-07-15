//! Route 数据访问层（v1.1.2 从 routes.rs 迁入）。
//!
//! 分组（routes）+ 分组项（route_items）CRUD，含 provider 自动识别（model_info 映射）。
//! UNIQUE/FOREIGN KEY 约束冲突以 `sqlx::Error` 上抛，由 handler 映射 ApiError。

use std::collections::HashMap;

use async_trait::async_trait;
use sqlx::{AssertSqlSafe, Row, SqlitePool};

use crate::domain::route::{
    AddRouteItemRequest, CreateRouteRequest, ListRoutesQuery, Route, RouteItem, RouteItemInfo,
    UpdateRouteRequest,
};
use crate::util::timeutil::tz_modifier;

#[async_trait]
pub trait RouteRepository: Send + Sync {
    /// 列表（搜索/筛选/排序/分页），返回 (items, total)
    async fn list(&self, query: ListRoutesQuery) -> Result<(Vec<Route>, i64), sqlx::Error>;
    /// 创建分组（事务：INSERT routes + 批量 INSERT route_items）
    async fn create(&self, req: CreateRouteRequest) -> Result<Route, sqlx::Error>;
    async fn get(&self, id: &str) -> Result<Option<Route>, sqlx::Error>;
    /// 更新分组（事务）。返回 None=分组不存在
    async fn update(&self, id: &str, req: UpdateRouteRequest) -> Result<Option<Route>, sqlx::Error>;
    /// 删除分组（事务：先 items 后 route）。返回是否实际删除
    async fn delete(&self, id: &str) -> Result<bool, sqlx::Error>;
    /// 添加分组项。返回 None=分组不存在
    async fn add_item(
        &self,
        route_id: &str,
        req: AddRouteItemRequest,
    ) -> Result<Option<RouteItem>, sqlx::Error>;
    /// 删除分组项。返回是否实际删除
    async fn delete_item(&self, route_id: &str, item_id: &str) -> Result<bool, sqlx::Error>;

    // ── proxy 热路径查询（轻量，不含 provider 解析；priority ASC 匹配 proxy 语义）──
    /// 按名称查启用的路由，返回 (id, name)。
    async fn find_enabled_by_name(&self, name: &str) -> Result<Option<(String, String)>, sqlx::Error>;
    /// 列出启用且带正则的路由 (id, name, match_regex)。
    async fn list_enabled_with_regex(
        &self,
    ) -> Result<Vec<(String, String, Option<String>)>, sqlx::Error>;
    /// 路由项（proxy 形状，priority ASC, weight DESC）。
    async fn list_route_items_for_proxy(
        &self,
        route_id: &str,
    ) -> Result<Vec<RouteItemInfo>, sqlx::Error>;

    /// 启用的路由名列表（/v1/models 列表用）。
    async fn list_enabled_names(&self) -> Result<Vec<String>, sqlx::Error>;
}

pub struct SqliteRouteRepository {
    pool: SqlitePool,
    timezone_offset: i32,
}

impl SqliteRouteRepository {
    pub fn new(pool: SqlitePool, timezone_offset: i32) -> Self {
        Self {
            pool,
            timezone_offset,
        }
    }

    fn tz_modifier(&self) -> String {
        tz_modifier(self.timezone_offset)
    }

    /// 动态 WHERE（搜索 + 状态筛选）
    fn push_where(builder: &mut sqlx::QueryBuilder<sqlx::Sqlite>, query: &ListRoutesQuery) -> bool {
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

    /// 取单个分组（含 items + provider 自动识别）
    async fn fetch_route(&self, id: &str) -> Result<Option<Route>, sqlx::Error> {
        let tz = self.tz_modifier();
        let result = sqlx::query_as::<_, (String, String, String, Option<String>, bool, i32, bool, String, String)>(
            AssertSqlSafe(format!("SELECT id, name, provider, match_regex, retry_enabled, first_token_timeout_secs, enabled, datetime(created_at, '{}') as created_at, datetime(updated_at, '{}') as updated_at FROM routes WHERE id = ?", tz, tz).as_str()),
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        let Some((id, name, stored_provider, match_regex, retry_enabled, first_token_timeout_secs, enabled, created_at, updated_at)) = result else {
            return Ok(None);
        };

        let items = self.fetch_route_items(&id).await?;
        let provider_map = self.load_model_provider_map().await?;
        let provider = resolve_provider(&stored_provider, &name, &provider_map);

        Ok(Some(Route {
            id,
            name,
            provider,
            match_regex,
            retry_enabled,
            first_token_timeout_secs,
            enabled,
            items,
            created_at,
            updated_at,
        }))
    }

    async fn fetch_route_items(&self, route_id: &str) -> Result<Vec<RouteItem>, sqlx::Error> {
        let items = sqlx::query_as::<_, (String, String, String, i32, i32)>(
            "SELECT id, channel_id, model_name, priority, weight FROM route_items WHERE route_id = ? ORDER BY priority DESC, weight DESC",
        )
        .bind(route_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(items
            .into_iter()
            .map(|(id, channel_id, model_name, priority, weight)| RouteItem {
                id,
                channel_id,
                model_name,
                priority,
                weight,
            })
            .collect())
    }

    /// 从 model_info 表加载 model -> provider 映射
    async fn load_model_provider_map(&self) -> Result<HashMap<String, String>, sqlx::Error> {
        let rows = sqlx::query_as::<_, (String, String)>(
            "SELECT model, provider FROM model_info WHERE provider != ''",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().collect())
    }
}

#[async_trait]
impl RouteRepository for SqliteRouteRepository {
    async fn list(&self, query: ListRoutesQuery) -> Result<(Vec<Route>, i64), sqlx::Error> {
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

        let mut count_builder = sqlx::QueryBuilder::new("SELECT COUNT(*) FROM routes");
        Self::push_where(&mut count_builder, &query);
        let count_row = count_builder.build().fetch_one(&self.pool).await?;
        let total: i64 = count_row.get(0);

        let tz = self.tz_modifier();
        let mut data_builder = sqlx::QueryBuilder::new(format!(
            "SELECT id, name, provider, match_regex, retry_enabled, first_token_timeout_secs, enabled, datetime(created_at, '{}') as created_at, datetime(updated_at, '{}') as updated_at FROM routes",
            tz, tz
        ));
        Self::push_where(&mut data_builder, &query);
        data_builder.push(format!(" ORDER BY {} {} ", order_field, order_dir));
        data_builder.push(" LIMIT ");
        data_builder.push_bind(page_size);
        data_builder.push(" OFFSET ");
        data_builder.push_bind(offset);

        let rows = data_builder.build().fetch_all(&self.pool).await?;

        // 批量加载 provider 映射，用于自动识别
        let provider_map = self.load_model_provider_map().await?;

        let mut items_list = Vec::new();
        for row in &rows {
            let id: String = row.get("id");
            let route_items = self.fetch_route_items(&id).await?;
            let stored_provider: String = row.get("provider");
            let name: String = row.get("name");
            let provider = resolve_provider(&stored_provider, &name, &provider_map);
            items_list.push(Route {
                id,
                name,
                provider,
                match_regex: row.get("match_regex"),
                retry_enabled: row.get("retry_enabled"),
                first_token_timeout_secs: row.get("first_token_timeout_secs"),
                enabled: row.get("enabled"),
                items: route_items,
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
            });
        }

        Ok((items_list, total))
    }

    async fn create(&self, req: CreateRouteRequest) -> Result<Route, sqlx::Error> {
        let route_id = crate::util::id::generate_id();
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            r#"INSERT INTO routes (id, name, provider, match_regex, retry_enabled, first_token_timeout_secs, enabled)
               VALUES (?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(&route_id)
        .bind(&req.name)
        .bind(req.provider.as_deref().unwrap_or(""))
        .bind(&req.match_regex)
        .bind(req.retry_enabled.unwrap_or(true))
        .bind(req.first_token_timeout_secs.unwrap_or(30))
        .bind(req.enabled.unwrap_or(true))
        .execute(&mut *tx)
        .await?;

        for item in &req.items {
            let item_id = crate::util::id::generate_id();
            sqlx::query(
                r#"INSERT INTO route_items (id, route_id, channel_id, model_name, priority, weight)
                   VALUES (?, ?, ?, ?, ?, ?)"#,
            )
            .bind(&item_id)
            .bind(&route_id)
            .bind(&item.channel_id)
            .bind(&item.model_name)
            .bind(item.priority.unwrap_or(1))
            .bind(item.weight.unwrap_or(100))
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;

        self.fetch_route(&route_id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
    }

    async fn get(&self, id: &str) -> Result<Option<Route>, sqlx::Error> {
        self.fetch_route(id).await
    }

    async fn update(&self, id: &str, req: UpdateRouteRequest) -> Result<Option<Route>, sqlx::Error> {
        let existing = sqlx::query_scalar::<_, String>("SELECT id FROM routes WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        if existing.is_none() {
            return Ok(None);
        }

        let mut tx = self.pool.begin().await?;

        let mut builder = sqlx::QueryBuilder::new("UPDATE routes SET ");
        let mut separated = builder.separated(", ");
        if let Some(ref name) = req.name {
            separated.push("name = ");
            separated.push_bind_unseparated(name);
        }
        if let Some(ref provider) = req.provider {
            separated.push("provider = ");
            separated.push_bind_unseparated(provider);
        }
        if let Some(ref regex) = req.match_regex {
            separated.push("match_regex = ");
            separated.push_bind_unseparated(regex);
        }
        if let Some(retry_enabled) = req.retry_enabled {
            separated.push("retry_enabled = ");
            separated.push_bind_unseparated(retry_enabled);
        }
        if let Some(timeout) = req.first_token_timeout_secs {
            separated.push("first_token_timeout_secs = ");
            separated.push_bind_unseparated(timeout);
        }
        if let Some(enabled) = req.enabled {
            separated.push("enabled = ");
            separated.push_bind_unseparated(enabled);
        }
        separated.push("updated_at = CURRENT_TIMESTAMP");
        builder.push(" WHERE id = ");
        builder.push_bind(id);
        builder.build().execute(&mut *tx).await?;

        if let Some(items) = &req.items {
            sqlx::query("DELETE FROM route_items WHERE route_id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            for item in items {
                let item_id = crate::util::id::generate_id();
                sqlx::query(
                    r#"INSERT INTO route_items (id, route_id, channel_id, model_name, priority, weight)
                       VALUES (?, ?, ?, ?, ?, ?)"#,
                )
                .bind(&item_id)
                .bind(id)
                .bind(&item.channel_id)
                .bind(&item.model_name)
                .bind(item.priority.unwrap_or(1))
                .bind(item.weight.unwrap_or(100))
                .execute(&mut *tx)
                .await?;
            }
        }

        tx.commit().await?;

        self.fetch_route(id).await
    }

    async fn delete(&self, id: &str) -> Result<bool, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM route_items WHERE route_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        let result = sqlx::query("DELETE FROM routes WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        let deleted = result.rows_affected() > 0;
        tx.commit().await?;
        Ok(deleted)
    }

    async fn add_item(
        &self,
        route_id: &str,
        req: AddRouteItemRequest,
    ) -> Result<Option<RouteItem>, sqlx::Error> {
        let existing = sqlx::query_scalar::<_, String>("SELECT id FROM routes WHERE id = ?")
            .bind(route_id)
            .fetch_optional(&self.pool)
            .await?;
        if existing.is_none() {
            return Ok(None);
        }

        let item_id = crate::util::id::generate_id();
        let priority = req.priority.unwrap_or(1);
        let weight = req.weight.unwrap_or(100);

        sqlx::query(
            r#"INSERT INTO route_items (id, route_id, channel_id, model_name, priority, weight)
               VALUES (?, ?, ?, ?, ?, ?)"#,
        )
        .bind(&item_id)
        .bind(route_id)
        .bind(&req.channel_id)
        .bind(&req.model_name)
        .bind(priority)
        .bind(weight)
        .execute(&self.pool)
        .await?;

        Ok(Some(RouteItem {
            id: item_id,
            channel_id: req.channel_id,
            model_name: req.model_name,
            priority,
            weight,
        }))
    }

    async fn delete_item(&self, route_id: &str, item_id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM route_items WHERE id = ? AND route_id = ?")
            .bind(item_id)
            .bind(route_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn find_enabled_by_name(&self, name: &str) -> Result<Option<(String, String)>, sqlx::Error> {
        sqlx::query_as::<_, (String, String)>(
            "SELECT id, name FROM routes WHERE name = ? AND enabled = 1",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
    }

    async fn list_enabled_with_regex(
        &self,
    ) -> Result<Vec<(String, String, Option<String>)>, sqlx::Error> {
        sqlx::query_as::<_, (String, String, Option<String>)>(
            "SELECT id, name, match_regex FROM routes WHERE enabled = 1 AND match_regex IS NOT NULL",
        )
        .fetch_all(&self.pool)
        .await
    }

    async fn list_route_items_for_proxy(
        &self,
        route_id: &str,
    ) -> Result<Vec<RouteItemInfo>, sqlx::Error> {
        let items = sqlx::query_as::<_, (String, String, i32, i32)>(
            "SELECT channel_id, model_name, priority, weight FROM route_items WHERE route_id = ? ORDER BY priority ASC, weight DESC",
        )
        .bind(route_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(items
            .into_iter()
            .map(|(channel_id, model_name, priority, weight)| RouteItemInfo {
                channel_id,
                model_name,
                priority,
                weight,
            })
            .collect())
    }

    async fn list_enabled_names(&self) -> Result<Vec<String>, sqlx::Error> {
        sqlx::query_scalar::<_, String>("SELECT name FROM routes WHERE enabled = 1")
            .fetch_all(&self.pool)
            .await
    }
}

/// 解析 provider：手动值优先，否则用分组名匹配 model_info
fn resolve_provider(
    stored: &str,
    group_name: &str,
    provider_map: &HashMap<String, String>,
) -> String {
    if !stored.is_empty() {
        return stored.to_string();
    }
    // 用分组名精确匹配 model_info.model
    if let Some(provider) = provider_map.get(group_name) {
        return provider.clone();
    }
    // 模糊匹配：分组名是 model name 的前缀（如 "gpt-4o" 匹配 "gpt-4o-2024-08-06"）
    for (model, provider) in provider_map {
        if model.starts_with(group_name) || group_name.starts_with(model) {
            return provider.clone();
        }
    }
    String::new()
}
