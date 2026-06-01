use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

use axum::{
    Json, RequestPartsExt,
    body::Body,
    extract::FromRequestParts,
    http::{Method, Request, StatusCode, header::CONTENT_TYPE, request::Parts},
    middleware::Next,
    response::IntoResponse,
};
use axum_extra::{
    TypedHeader,
    headers::{Authorization, authorization::Bearer},
};
use sqlx::SqlitePool;

use crate::api::ApiError;
use crate::auth::decode_jwt;

/// 缓存条目 TTL（秒）
const CACHE_TTL_SECS: u64 = 300;

/// API Key 缓存
#[derive(Clone)]
pub struct ApiKeyCache {
    keys: Arc<RwLock<HashMap<String, ApiKeyEntry>>>,
}

#[derive(Clone)]
struct ApiKeyEntry {
    id: String,
    name: String,
    enabled: bool,
    cached_at: Instant,
}

impl Default for ApiKeyCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ApiKeyCache {
    pub fn new() -> Self {
        Self {
            keys: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 获取缓存的 API Key（过期返回 None）
    async fn get(&self, key: &str) -> Option<(String, String, bool)> {
        let cache = self.keys.read().await;
        cache
            .get(key)
            .filter(|e| e.cached_at.elapsed().as_secs() < CACHE_TTL_SECS)
            .map(|e| (e.id.clone(), e.name.clone(), e.enabled))
    }

    /// 设置 API Key 缓存
    async fn set(&self, key: String, id: String, name: String, enabled: bool) {
        let mut cache = self.keys.write().await;
        if cache.len() >= 1000 {
            cache.retain(|_, e| e.cached_at.elapsed().as_secs() < CACHE_TTL_SECS);
        }
        cache.insert(
            key,
            ApiKeyEntry {
                id,
                name,
                enabled,
                cached_at: Instant::now(),
            },
        );
    }
}

/// 从请求中提取 Claims（管理 API 认证）
pub struct AuthClaims(pub crate::auth::Claims);

impl<S: Send + Sync> FromRequestParts<S> for AuthClaims {
    type Rejection = (StatusCode, String);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // 提取 Authorization header
        let TypedHeader(Authorization(bearer)) = parts
            .extract::<TypedHeader<Authorization<Bearer>>>()
            .await
            .map_err(|_| (StatusCode::UNAUTHORIZED, "缺少认证令牌".to_string()))?;

        // 从 extensions 获取 JWT secret
        let jwt_secret = parts.extensions.get::<String>().ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "JWT 配置缺失".to_string(),
            )
        })?;

        let claims = decode_jwt(bearer.token(), jwt_secret)
            .map_err(|_| (StatusCode::UNAUTHORIZED, "无效的认证令牌".to_string()))?;

        Ok(AuthClaims(claims))
    }
}

/// API Key 认证结果（代理 API 认证）
pub struct ApiKeyAuth {
    pub key_id: String,
}

impl<S: Send + Sync> FromRequestParts<S> for ApiKeyAuth {
    type Rejection = (StatusCode, axum::Json<serde_json::Value>);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // 优先从 Authorization: Bearer 提取，回退到 x-api-key（Anthropic 兼容）
        let api_key = match parts.extract::<TypedHeader<Authorization<Bearer>>>().await {
            Ok(TypedHeader(Authorization(bearer))) => bearer.token().to_string(),
            Err(_) => parts
                .headers
                .get("x-api-key")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
                .unwrap_or_default(),
        };

        if api_key.is_empty() {
            return Err((
                StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({
                    "error": { "message": "缺少 API Key", "type": "authentication_error" }
                })),
            ));
        }

        // 1. 检查缓存
        if let Some(cache) = parts.extensions.get::<ApiKeyCache>()
            && let Some((id, _name, enabled)) = cache.get(&api_key).await
        {
            if !enabled {
                return Err((
                    StatusCode::FORBIDDEN,
                    axum::Json(serde_json::json!({
                        "error": { "message": "API Key 已禁用", "type": "authentication_error" }
                    })),
                ));
            }
            return Ok(ApiKeyAuth {
                key_id: id,
            });
        }

        // 2. 缓存未命中，查询数据库
        let pool = parts.extensions.get::<SqlitePool>().ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({
                    "error": { "message": "数据库配置缺失", "type": "server_error" }
                })),
            )
        })?;

        let result = sqlx::query_as::<_, (String, String, bool)>(
            "SELECT id, name, enabled FROM api_keys WHERE api_key = ?",
        )
        .bind(&api_key)
        .fetch_optional(pool)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({
                    "error": { "message": "数据库查询失败", "type": "server_error" }
                })),
            )
        })?;

        match result {
            Some((id, name, enabled)) => {
                if let Some(cache) = parts.extensions.get::<ApiKeyCache>() {
                    cache.set(api_key, id.clone(), name.clone(), enabled).await;
                }
                if !enabled {
                    return Err((
                        StatusCode::FORBIDDEN,
                        axum::Json(serde_json::json!({
                            "error": { "message": "API Key 已禁用", "type": "authentication_error" }
                        })),
                    ));
                }
                Ok(ApiKeyAuth {
                    key_id: id,
                })
            }
            None => Err((
                StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({
                    "error": { "message": "无效的 API Key", "type": "authentication_error" }
                })),
            )),
        }
    }
}

/// 管理 API 认证中间件
pub async fn require_admin_auth(
    request: Request<Body>,
    next: Next,
) -> Result<axum::response::Response, axum::response::Response> {
    let jwt_secret = request
        .extensions()
        .get::<String>()
        .cloned()
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({"code": 500, "message": "JWT 配置缺失"})),
            )
                .into_response()
        })?;

    let token = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({"code": 401, "message": "缺少认证令牌"})),
            )
                .into_response()
        })?;

    decode_jwt(token, &jwt_secret).map_err(|_| {
        (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({"code": 401, "message": "无效的认证令牌"})),
        )
            .into_response()
    })?;

    Ok(next.run(request).await)
}

/// Content-Type 校验中间件：POST/PUT/PATCH 必须是 application/json
pub async fn require_json(request: Request<Body>, next: Next) -> impl IntoResponse {
    let method = request.method();
    if matches!(
        *method,
        Method::GET | Method::DELETE | Method::OPTIONS | Method::HEAD
    ) {
        return next.run(request).await;
    }
    let ct = request
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if ct.contains("application/json") {
        next.run(request).await
    } else {
        (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Json(ApiError::new(415, "Content-Type 必须是 application/json")),
        )
            .into_response()
    }
}
