use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

pub mod cors;

use axum::{
    Json, RequestPartsExt,
    body::Body,
    extract::FromRequestParts,
    http::{Method, Request, StatusCode, header::CONTENT_TYPE, request::Parts},
    middleware::Next,
    response::IntoResponse,
};

use crate::repository::api_key_repository::{ApiKeyRepository, SqliteApiKeyRepository};
use axum_extra::{
    TypedHeader,
    headers::{Authorization, authorization::Bearer},
};
use sqlx::SqlitePool;

use crate::error::app::ApiError;
use crate::auth::decode_jwt;
use crate::service::stats::recorder::{RequestRecord, StatsRecorder};

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
    rate_limit_rpm: u64,
    rate_limit_tpm: u64,
    allowed_routes: String,
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
    async fn get(&self, key: &str) -> Option<(String, String, bool, u64, u64, String)> {
        let cache = self.keys.read().await;
        cache
            .get(key)
            .filter(|e| e.cached_at.elapsed().as_secs() < CACHE_TTL_SECS)
            .map(|e| {
                (
                    e.id.clone(),
                    e.name.clone(),
                    e.enabled,
                    e.rate_limit_rpm,
                    e.rate_limit_tpm,
                    e.allowed_routes.clone(),
                )
            })
    }

    /// 设置 API Key 缓存
    #[allow(clippy::too_many_arguments)]
    async fn set(
        &self,
        key: String,
        id: String,
        name: String,
        enabled: bool,
        rate_limit_rpm: u64,
        rate_limit_tpm: u64,
        allowed_routes: String,
    ) {
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
                rate_limit_rpm,
                rate_limit_tpm,
                allowed_routes,
                cached_at: Instant::now(),
            },
        );
    }

    /// 清除指定 API Key 的缓存
    pub async fn invalidate(&self, key: &str) {
        let mut cache = self.keys.write().await;
        cache.remove(key);
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

/// 鉴权失败时补一条 usage_logs（审计兜底）。
///
/// 背景：ApiKeyAuth 是 FromRequestParts 提取器，鉴权失败时直接返回错误响应，
/// 不会进入 handler，也不会触发 proxy 层的 save_request_record，
/// 导致"CC 已失败但 usage_logs 缺失"。本函数在每个失败分支显式调用，覆盖审计盲区。
///
/// 注：Parts 不携带 body（body 在 Request 里，Parts 只是 header 等元数据），
/// 所以 request_content 为 None；model 从 URI 推断（代理路由不含 model 字段）。
async fn record_auth_failure(
    parts: &Parts,
    error_kind: &str,
    error_message: &str,
    status_code: StatusCode,
) {
    let Some(recorder) = parts.extensions.get::<StatsRecorder>() else {
        return; // 提取器未注入（不应发生），直接放弃，避免影响正常鉴权返回
    };

    let request_id = crate::util::id::generate_id();
    let model = infer_model_from_uri(&parts.uri);
    let is_stream = false; // 鉴权阶段尚未解析 body，保守标记为非流式
    let user_agent = parts
        .headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let method = parts.method.as_str().to_string();
    let path = parts.uri.path().to_string();

    let record = RequestRecord {
        request_id: Some(request_id),
        api_key_id: None,
        channel_id: None,
        route_id: None,
        requested_model: model,
        actual_model: None,
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        cache_creation_tokens: 0,
        cost: None,
        latency_ms: None,
        ttft_ms: None,
        status_code: Some(status_code.as_u16() as i32),
        error_message: Some(format!("[{}] {}", error_kind, error_message)),
        endpoint_type: None,
        request_type: "auth_failure".to_string(),
        request_content: Some(format!("{} {}", method, path)),
        response_content: None,
        is_stream,
        upstream_key_hint: None,
        attempts: vec![],
        user_agent,
    };
    let _ = recorder.record_request(record).await;
}

/// 从代理 URI 推断客户端请求的协议类型（作为 requested_model 占位）。
///
/// 代理路由形如 `/v1/chat/completions`、`/v1/responses`、`/v1/messages` 等，
/// 鉴权失败时尚未解析 body 拿不到真实 model 名，用 endpoint 路径作为可审计标识。
fn infer_model_from_uri(uri: &axum::http::Uri) -> String {
    let path = uri.path();
    // 取 URI 最后一段作为 endpoint 标识
    path.rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .map(|s| format!("proxy:{}", s))
        .unwrap_or_else(|| "proxy:unknown".to_string())
}

/// API Key 认证结果（代理 API 认证）
pub struct ApiKeyAuth {
    pub key_id: String,
    pub rate_limit_rpm: u64,
    pub rate_limit_tpm: u64,
    pub allowed_routes: String,
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
            record_auth_failure(
                parts,
                "missing_api_key",
                "缺少 API Key",
                StatusCode::UNAUTHORIZED,
            )
            .await;
            return Err((
                StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({
                    "error": { "message": "缺少 API Key", "type": "authentication_error" }
                })),
            ));
        }

        // 1. 检查缓存
        if let Some(cache) = parts.extensions.get::<ApiKeyCache>()
            && let Some((id, _name, enabled, rpm, tpm, routes)) = cache.get(&api_key).await
        {
            if !enabled {
                record_auth_failure(
                    parts,
                    "api_key_disabled",
                    "API Key 已禁用（缓存命中）",
                    StatusCode::FORBIDDEN,
                )
                .await;
                return Err((
                    StatusCode::FORBIDDEN,
                    axum::Json(serde_json::json!({
                        "error": { "message": "API Key 已禁用", "type": "authentication_error" }
                    })),
                ));
            }
            return Ok(ApiKeyAuth {
                key_id: id,
                rate_limit_rpm: rpm,
                rate_limit_tpm: tpm,
                allowed_routes: routes,
            });
        }

        // 2. 缓存未命中，查询数据库
        let pool = match parts.extensions.get::<SqlitePool>() {
            Some(p) => p,
            None => {
                record_auth_failure(
                    parts,
                    "server_misconfigured",
                    "数据库配置缺失",
                    StatusCode::INTERNAL_SERVER_ERROR,
                )
                .await;
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    axum::Json(serde_json::json!({
                        "error": { "message": "数据库配置缺失", "type": "server_error" }
                    })),
                ));
            }
        };

        // tz 不影响 find_for_auth（无 datetime 列），传 0
        let query_result =
            SqliteApiKeyRepository::new(pool.clone(), 0).find_for_auth(&api_key).await;

        let result = match query_result {
            Ok(r) => r,
            Err(e) => {
                record_auth_failure(
                    parts,
                    "db_query_failed",
                    &format!("数据库查询失败: {}", e),
                    StatusCode::INTERNAL_SERVER_ERROR,
                )
                .await;
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    axum::Json(serde_json::json!({
                        "error": { "message": "数据库查询失败", "type": "server_error" }
                    })),
                ));
            }
        };

        match result {
            Some((id, name, enabled, rpm, tpm, routes)) => {
                let rpm = rpm as u64;
                let tpm = tpm as u64;
                if let Some(cache) = parts.extensions.get::<ApiKeyCache>() {
                    cache
                        .set(
                            api_key,
                            id.clone(),
                            name.clone(),
                            enabled,
                            rpm,
                            tpm,
                            routes.clone(),
                        )
                        .await;
                }
                if !enabled {
                    record_auth_failure(
                        parts,
                        "api_key_disabled",
                        "API Key 已禁用（DB 命中）",
                        StatusCode::FORBIDDEN,
                    )
                    .await;
                    return Err((
                        StatusCode::FORBIDDEN,
                        axum::Json(serde_json::json!({
                            "error": { "message": "API Key 已禁用", "type": "authentication_error" }
                        })),
                    ));
                }
                Ok(ApiKeyAuth {
                    key_id: id,
                    rate_limit_rpm: rpm,
                    rate_limit_tpm: tpm,
                    allowed_routes: routes,
                })
            }
            None => {
                record_auth_failure(
                    parts,
                    "invalid_api_key",
                    "无效的 API Key",
                    StatusCode::UNAUTHORIZED,
                )
                .await;
                Err((
                    StatusCode::UNAUTHORIZED,
                    axum::Json(serde_json::json!({
                        "error": { "message": "无效的 API Key", "type": "authentication_error" }
                    })),
                ))
            }
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
