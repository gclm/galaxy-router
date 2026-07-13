use axum::{Json, extract::State, response::IntoResponse};
use sqlx::SqlitePool;

use crate::api::handlers::admin::api_keys::parse_supported_models;
use crate::api::middleware::ApiKeyAuth;

/// 获取可用模型列表（支持 API Key 分组权限和模型过滤）
pub async fn list(auth: ApiKeyAuth, State(pool): State<SqlitePool>) -> impl IntoResponse {
    // 获取所有启用的分组名
    let all_groups = sqlx::query_scalar::<_, String>("SELECT name FROM routes WHERE enabled = 1")
        .fetch_all(&pool)
        .await
        .unwrap_or_default();

    let models: Vec<String> = if !auth.allowed_routes.is_empty() {
        // 优先使用 allowed_routes 过滤：Key 只能访问指定的分组
        let allowed: Vec<String> = parse_supported_models(&auth.allowed_routes);
        all_groups
            .into_iter()
            .filter(|g| allowed.iter().any(|a| a.eq_ignore_ascii_case(g)))
            .collect()
    } else {
        // 回退到 supported_models（传统行为）
        let supported = get_supported_models(&pool, &auth.key_id).await;
        if let Some(supported) = supported {
            all_groups
                .into_iter()
                .filter(|g| supported.contains(g))
                .collect()
        } else {
            all_groups
        }
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

/// 获取 API Key 的支持模型列表（传统方式）
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
