use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, SqlitePool};

use crate::api::handlers::admin::channels::PaginatedResponse;
use crate::api::{ApiError, ApiResponse, response::generate_id};
use crate::stats::tz_modifier;

/// 分组
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub match_regex: Option<String>,
    pub retry_enabled: bool,
    pub max_retries: i32,
    pub first_token_timeout_secs: i32,
    pub enabled: bool,
    pub items: Vec<GroupItem>,
    pub created_at: String,
    pub updated_at: String,
}

/// 分组项
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GroupItem {
    pub id: String,
    pub channel_id: String,
    pub model_name: String,
    pub priority: i32,
    pub weight: i32,
}

/// 列表查询参数
#[derive(Debug, Deserialize)]
pub struct ListGroupsQuery {
    pub search: Option<String>,
    pub status: Option<String>,
    pub sort_by: Option<String>,
    pub sort_order: Option<String>,
    pub page: Option<i32>,
    pub page_size: Option<i32>,
}

/// 创建分组请求
#[derive(Debug, Deserialize)]
pub struct CreateGroupRequest {
    pub name: String,
    pub provider: Option<String>,
    pub match_regex: Option<String>,
    pub retry_enabled: Option<bool>,
    pub max_retries: Option<i32>,
    pub first_token_timeout_secs: Option<i32>,
    pub enabled: Option<bool>,
    pub items: Vec<CreateGroupItemRequest>,
}

/// 创建分组项请求
#[derive(Debug, Deserialize)]
pub struct CreateGroupItemRequest {
    pub channel_id: String,
    pub model_name: String,
    pub priority: Option<i32>,
    pub weight: Option<i32>,
}

/// 更新分组请求
#[derive(Debug, Deserialize)]
pub struct UpdateGroupRequest {
    pub name: Option<String>,
    pub provider: Option<String>,
    pub match_regex: Option<String>,
    pub retry_enabled: Option<bool>,
    pub max_retries: Option<i32>,
    pub first_token_timeout_secs: Option<i32>,
    pub enabled: Option<bool>,
    pub items: Option<Vec<CreateGroupItemRequest>>,
}

/// 添加分组项请求
#[derive(Debug, Deserialize)]
pub struct AddGroupItemRequest {
    pub channel_id: String,
    pub model_name: String,
    pub priority: Option<i32>,
    pub weight: Option<i32>,
}

/// 分组状态
#[derive(Clone)]
pub struct GroupState {
    pub pool: SqlitePool,
    pub cache: crate::proxy::ProxyCache,
    pub timezone_offset: i32,
}

/// 获取分组列表（支持搜索、筛选、排序、分页）
pub async fn list(
    State(state): State<GroupState>,
    Query(query): Query<ListGroupsQuery>,
) -> Result<Json<ApiResponse<PaginatedResponse<Group>>>, (StatusCode, Json<ApiError>)> {
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

    let mut count_builder = sqlx::QueryBuilder::new("SELECT COUNT(*) FROM groups");
    let _has_where = push_where(&mut count_builder, &query);

    let count_row = count_builder
        .build()
        .fetch_one(&state.pool)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;
    let total: i64 = sqlx::Row::get(&count_row, 0);

    let tz = tz_modifier(state.timezone_offset);
    let mut data_builder = sqlx::QueryBuilder::new(format!(
        "SELECT id, name, provider, match_regex, retry_enabled, max_retries, first_token_timeout_secs, enabled, datetime(created_at, '{}') as created_at, datetime(updated_at, '{}') as updated_at FROM groups",
        tz, tz
    ));
    push_where(&mut data_builder, &query);
    data_builder.push(format!(" ORDER BY {} {} ", order_field, order_dir));
    data_builder.push(" LIMIT ");
    data_builder.push_bind(page_size);
    data_builder.push(" OFFSET ");
    data_builder.push_bind(offset);

    let rows = data_builder
        .build()
        .fetch_all(&state.pool)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    // 批量加载 model_info 的 provider 映射，用于自动识别
    let provider_map = load_model_provider_map(&state.pool).await?;

    use sqlx::Row;
    let mut items_list = Vec::new();
    for row in &rows {
        let id: String = row.get("id");
        let group_items = get_group_items(&state.pool, &id).await?;
        let stored_provider: String = row.get("provider");
        let name: String = row.get("name");
        let provider = resolve_provider(&stored_provider, &name, &provider_map);
        items_list.push(Group {
            id,
            name,
            provider,
            match_regex: row.get("match_regex"),
            retry_enabled: row.get("retry_enabled"),
            max_retries: row.get("max_retries"),
            first_token_timeout_secs: row.get("first_token_timeout_secs"),
            enabled: row.get("enabled"),
            items: group_items,
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        });
    }

    Ok(Json(ApiResponse::success(PaginatedResponse {
        items: items_list,
        total,
    })))
}

fn push_where(builder: &mut sqlx::QueryBuilder<sqlx::Sqlite>, query: &ListGroupsQuery) -> bool {
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

/// 创建分组
pub async fn create(
    State(state): State<GroupState>,
    Json(req): Json<CreateGroupRequest>,
) -> Result<(StatusCode, Json<ApiResponse<Group>>), (StatusCode, Json<ApiError>)> {
    if req.name.is_empty() {
        return Err(ApiError::bad_request("分组名称不能为空"));
    }
    if req.items.is_empty() {
        return Err(ApiError::bad_request("至少需要一个分组项"));
    }

    let group_id = generate_id();

    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    sqlx::query(
        r#"
        INSERT INTO groups (id, name, provider, match_regex, retry_enabled, max_retries, first_token_timeout_secs, enabled)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        "#
    )
    .bind(&group_id)
    .bind(&req.name)
    .bind(req.provider.as_deref().unwrap_or(""))
    .bind(&req.match_regex)
    .bind(req.retry_enabled.unwrap_or(true))
    .bind(req.max_retries.unwrap_or(3))
    .bind(req.first_token_timeout_secs.unwrap_or(30))
    .bind(req.enabled.unwrap_or(true))
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        if e.to_string().contains("UNIQUE constraint failed") {
            ApiError::conflict("分组名称已存在")
        } else {
            ApiError::internal_error(e.to_string())
        }
    })?;

    for item in &req.items {
        let item_id = generate_id();
        sqlx::query(
            r#"
            INSERT INTO group_items (id, group_id, channel_id, model_name, priority, weight)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&item_id)
        .bind(&group_id)
        .bind(&item.channel_id)
        .bind(&item.model_name)
        .bind(item.priority.unwrap_or(1))
        .bind(item.weight.unwrap_or(100))
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            if e.to_string().contains("FOREIGN KEY constraint failed") {
                ApiError::bad_request("渠道不存在")
            } else {
                ApiError::internal_error(e.to_string())
            }
        })?;
    }

    tx.commit()
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    state.cache.invalidate_all_groups().await;
    let group = get_group_by_id(&state.pool, &group_id, state.timezone_offset).await?;
    Ok((StatusCode::CREATED, Json(ApiResponse::success(group))))
}

/// 获取单个分组
pub async fn get(
    State(state): State<GroupState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<Group>>, (StatusCode, Json<ApiError>)> {
    let group = get_group_by_id(&state.pool, &id, state.timezone_offset).await?;
    Ok(Json(ApiResponse::success(group)))
}

/// 更新分组
pub async fn update(
    State(state): State<GroupState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateGroupRequest>,
) -> Result<Json<ApiResponse<Group>>, (StatusCode, Json<ApiError>)> {
    let existing = sqlx::query_scalar::<_, String>("SELECT id FROM groups WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.pool)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    if existing.is_none() {
        return Err(ApiError::not_found("分组不存在"));
    }

    if let Some(items) = &req.items
        && items.is_empty()
    {
        return Err(ApiError::bad_request("至少需要一个分组项"));
    }

    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    let mut builder = sqlx::QueryBuilder::new("UPDATE groups SET ");
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
    if let Some(max_retries) = req.max_retries {
        separated.push("max_retries = ");
        separated.push_bind_unseparated(max_retries);
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
    builder.push_bind(&id);

    builder
        .build()
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    if let Some(items) = &req.items {
        sqlx::query("DELETE FROM group_items WHERE group_id = ?")
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(|e| ApiError::internal_error(e.to_string()))?;

        for item in items {
            let item_id = generate_id();
            sqlx::query(
                r#"
                INSERT INTO group_items (id, group_id, channel_id, model_name, priority, weight)
                VALUES (?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&item_id)
            .bind(&id)
            .bind(&item.channel_id)
            .bind(&item.model_name)
            .bind(item.priority.unwrap_or(1))
            .bind(item.weight.unwrap_or(100))
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                if e.to_string().contains("FOREIGN KEY constraint failed") {
                    ApiError::bad_request("渠道不存在")
                } else if e.to_string().contains("UNIQUE constraint failed") {
                    ApiError::conflict("分组项重复")
                } else {
                    ApiError::internal_error(e.to_string())
                }
            })?;
        }
    }

    tx.commit()
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    let group = get_group_by_id(&state.pool, &id, state.timezone_offset).await?;
    state.cache.invalidate_all_groups().await;
    Ok(Json(ApiResponse::success(group)))
}

/// 删除分组
pub async fn delete(
    State(state): State<GroupState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, (StatusCode, Json<ApiError>)> {
    let result = sqlx::query("DELETE FROM groups WHERE id = ?")
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("分组不存在"));
    }

    state.cache.invalidate_all_groups().await;
    Ok(Json(crate::api::response::success_empty()))
}

/// 添加分组项
pub async fn add_item(
    State(state): State<GroupState>,
    Path(id): Path<String>,
    Json(req): Json<AddGroupItemRequest>,
) -> Result<(StatusCode, Json<ApiResponse<GroupItem>>), (StatusCode, Json<ApiError>)> {
    let existing = sqlx::query_scalar::<_, String>("SELECT id FROM groups WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.pool)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    if existing.is_none() {
        return Err(ApiError::not_found("分组不存在"));
    }

    let item_id = generate_id();

    sqlx::query(
        r#"
        INSERT INTO group_items (id, group_id, channel_id, model_name, priority, weight)
        VALUES (?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&item_id)
    .bind(&id)
    .bind(&req.channel_id)
    .bind(&req.model_name)
    .bind(req.priority.unwrap_or(1))
    .bind(req.weight.unwrap_or(100))
    .execute(&state.pool)
    .await
    .map_err(|e| {
        if e.to_string().contains("FOREIGN KEY constraint failed") {
            ApiError::bad_request("渠道不存在")
        } else {
            ApiError::internal_error(e.to_string())
        }
    })?;

    let item = GroupItem {
        id: item_id,
        channel_id: req.channel_id,
        model_name: req.model_name,
        priority: req.priority.unwrap_or(1),
        weight: req.weight.unwrap_or(100),
    };

    Ok((StatusCode::CREATED, Json(ApiResponse::success(item))))
}

/// 删除分组项
pub async fn delete_item(
    State(state): State<GroupState>,
    Path((group_id, item_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<()>>, (StatusCode, Json<ApiError>)> {
    let result = sqlx::query("DELETE FROM group_items WHERE id = ? AND group_id = ?")
        .bind(&item_id)
        .bind(&group_id)
        .execute(&state.pool)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("分组项不存在"));
    }

    state.cache.invalidate_all_groups().await;
    Ok(Json(crate::api::response::success_empty()))
}

async fn get_group_by_id(
    pool: &SqlitePool,
    id: &str,
    timezone_offset: i32,
) -> Result<Group, (StatusCode, Json<ApiError>)> {
    let tz = tz_modifier(timezone_offset);
    let result = sqlx::query_as::<_, (String, String, String, Option<String>, bool, i32, i32, bool, String, String)>(
        AssertSqlSafe(format!("SELECT id, name, provider, match_regex, retry_enabled, max_retries, first_token_timeout_secs, enabled, datetime(created_at, '{}') as created_at, datetime(updated_at, '{}') as updated_at FROM groups WHERE id = ?", tz, tz).as_str())
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::internal_error(e.to_string()))?;

    let (
        id,
        name,
        stored_provider,
        match_regex,
        retry_enabled,
        max_retries,
        first_token_timeout_secs,
        enabled,
        created_at,
        updated_at,
    ) = result.ok_or_else(|| ApiError::not_found("分组不存在"))?;

    let items = get_group_items(pool, &id).await?;

    let provider_map = load_model_provider_map(pool).await?;
    let provider = resolve_provider(&stored_provider, &name, &provider_map);

    Ok(Group {
        id,
        name,
        provider,
        match_regex,
        retry_enabled,
        max_retries,
        first_token_timeout_secs,
        enabled,
        items,
        created_at,
        updated_at,
    })
}

async fn get_group_items(
    pool: &SqlitePool,
    group_id: &str,
) -> Result<Vec<GroupItem>, (StatusCode, Json<ApiError>)> {
    let items = sqlx::query_as::<_, (String, String, String, i32, i32)>(
        "SELECT id, channel_id, model_name, priority, weight FROM group_items WHERE group_id = ? ORDER BY priority DESC, weight DESC"
    )
    .bind(group_id)
    .fetch_all(pool)
    .await
    .map_err(|e| ApiError::internal_error(e.to_string()))?;

    Ok(items
        .into_iter()
        .map(|(id, channel_id, model_name, priority, weight)| GroupItem {
            id,
            channel_id,
            model_name,
            priority,
            weight,
        })
        .collect())
}

/// 从 model_info 表加载 model -> provider 映射
async fn load_model_provider_map(
    pool: &SqlitePool,
) -> Result<std::collections::HashMap<String, String>, (StatusCode, Json<ApiError>)> {
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT model, provider FROM model_info WHERE provider != ''",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| ApiError::internal_error(e.to_string()))?;

    Ok(rows.into_iter().collect())
}

/// 解析 provider：手动值优先，否则用 group name 匹配 model_info
fn resolve_provider(
    stored: &str,
    group_name: &str,
    provider_map: &std::collections::HashMap<String, String>,
) -> String {
    if !stored.is_empty() {
        return stored.to_string();
    }
    // 用分组名精确匹配 model_info.model
    if let Some(provider) = provider_map.get(group_name) {
        return provider.clone();
    }
    // 模糊匹配：group name 是 model name 的前缀（如 "gpt-4o" 匹配 "gpt-4o-2024-08-06"）
    for (model, provider) in provider_map {
        if model.starts_with(group_name) || group_name.starts_with(model) {
            return provider.clone();
        }
    }
    String::new()
}
