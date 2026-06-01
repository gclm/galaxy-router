use axum::{Json, extract::State, response::IntoResponse};
use sqlx::SqlitePool;

use crate::api::handlers::admin::api_keys::parse_supported_models;
use crate::api::middleware::ApiKeyAuth;

/// 获取可用模型列表（仅分组名，支持 API Key 模型过滤）
pub async fn list(auth: ApiKeyAuth, State(pool): State<SqlitePool>) -> impl IntoResponse {
    let groups = sqlx::query_scalar::<_, String>("SELECT name FROM groups WHERE enabled = 1")
        .fetch_all(&pool)
        .await
        .unwrap_or_default();

    let supported_models = get_supported_models(&pool, &auth.key_id).await;

    let models: Vec<String> = if let Some(supported) = supported_models {
        groups
            .into_iter()
            .filter(|g| supported.contains(g))
            .collect()
    } else {
        groups
    };

    let data: Vec<serde_json::Value> = models
        .into_iter()
        .map(|name| {
            serde_json::json!({
                "id": name,
                "object": "model",
                "created": 0,
                "owned_by": "galaxy-router"
            })
        })
        .collect();

    Json(serde_json::json!({
        "object": "list",
        "data": data
    }))
    .into_response()
}

/// 获取 API Key 的支持模型列表
async fn get_supported_models(pool: &SqlitePool, key_id: &str) -> Option<Vec<String>> {
    let result =
        sqlx::query_scalar::<_, String>("SELECT supported_models FROM api_keys WHERE id = ?")
            .bind(key_id)
            .fetch_optional(pool)
            .await
            .ok()??;

    if result.is_empty() {
        return None;
    }

    Some(parse_supported_models(&result))
}
