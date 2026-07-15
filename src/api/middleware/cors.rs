use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use sqlx::SqlitePool;

use crate::repository::settings_repository::{SettingsRepository, SqliteSettingsRepository};

/// 从数据库读取 CORS 允许的 origins
/// 返回值：空=禁止跨域，"*"=允许所有，其他=逗号分隔的白名单
async fn load_cors_origins(pool: &SqlitePool) -> String {
    SqliteSettingsRepository::new(pool.clone())
        .get("cors.allow_origins")
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// 检查 origin 是否在允许列表中
fn is_origin_allowed(origin: &str, allowed: &str) -> bool {
    let allowed = allowed.trim();

    if allowed.is_empty() {
        return false; // 空 = 禁止跨域
    }

    if allowed == "*" {
        return true; // * = 允许所有
    }

    let origin = origin.trim();
    if origin.is_empty() {
        return false;
    }

    // 提取 origin 的 host 部分用于匹配
    let origin_host = {
        let host = if let Some(idx) = origin.find("://") {
            &origin[idx + 3..]
        } else {
            origin
        };
        host.trim_end_matches('/')
    };

    // 匹配白名单列表（支持完整 origin 或仅域名）
    for item in allowed.split(',') {
        let item = item.trim().trim_end_matches('/');
        if item.is_empty() {
            continue;
        }
        if item == origin || item == origin_host {
            return true;
        }
    }

    false
}

/// 设置 CORS 响应头
fn set_cors_headers(headers: &mut HeaderMap, origin: &str) {
    // Access-Control-Allow-Origin 必须与请求的 Origin 一致（不能用 * 当 allow_credentials=true）
    let _ = headers.insert(
        "access-control-allow-origin",
        HeaderValue::from_str(origin).unwrap_or_else(|_| HeaderValue::from_static("*")),
    );
    let _ = headers.insert(
        "access-control-allow-credentials",
        HeaderValue::from_static("true"),
    );
    let _ = headers.insert(
        "access-control-allow-methods",
        HeaderValue::from_static("GET, POST, PUT, DELETE, OPTIONS"),
    );
    let _ = headers.insert(
        "access-control-allow-headers",
        HeaderValue::from_static("*"),
    );
    let _ = headers.insert(
        "access-control-expose-headers",
        HeaderValue::from_static("Content-Disposition"),
    );
}

/// CORS 中间件：异步从 DB 读取配置，动态校验 origin
pub async fn require_cors(pool: SqlitePool, request: Request<Body>, next: Next) -> Response {
    // 仅处理携带 Origin header 的请求（跨域请求）
    let origin = match request
        .headers()
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok())
    {
        Some(o) => o.to_string(),
        None => return next.run(request).await, // 无 Origin，非跨域，直接放行
    };

    // 从数据库加载 CORS 配置
    let allowed_origins = load_cors_origins(&pool).await;

    // 校验 origin
    if !is_origin_allowed(&origin, &allowed_origins) {
        // 跨域被拒绝：如果是 OPTIONS 预检，返回 204 不带 CORS 头
        // 如果是实际请求，返回 403
        if request.method() == Method::OPTIONS {
            return Response::builder()
                .status(StatusCode::NO_CONTENT)
                .body(Body::empty())
                .expect("valid empty response");
        }
        return Response::builder()
            .status(StatusCode::FORBIDDEN)
            .body(Body::from("Cross-origin request blocked"))
            .expect("valid response");
    }

    // 处理 OPTIONS 预检请求
    if request.method() == Method::OPTIONS {
        let mut response = Response::builder()
            .status(StatusCode::NO_CONTENT)
            .body(Body::empty())
            .expect("valid empty response");
        set_cors_headers(response.headers_mut(), &origin);
        return response;
    }

    // 实际请求：放行并附加 CORS 头
    let mut response = next.run(request).await;
    set_cors_headers(response.headers_mut(), &origin);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_origin_allowed_empty_forbids() {
        assert!(!is_origin_allowed("https://example.com", ""));
    }

    #[test]
    fn test_is_origin_allowed_wildcard_allows_all() {
        assert!(is_origin_allowed("https://example.com", "*"));
        assert!(is_origin_allowed("http://localhost:3000", "*"));
    }

    #[test]
    fn test_is_origin_allowed_whitelist_exact() {
        let whitelist = "https://example.com,https://app.example.com";
        assert!(is_origin_allowed("https://example.com", whitelist));
        assert!(is_origin_allowed("https://app.example.com", whitelist));
        assert!(!is_origin_allowed("https://other.com", whitelist));
    }

    #[test]
    fn test_is_origin_allowed_domain_only_match() {
        let whitelist = "example.com";
        assert!(is_origin_allowed("https://example.com", whitelist));
        assert!(is_origin_allowed("http://example.com", whitelist));
        assert!(!is_origin_allowed("https://evil.com", whitelist));
    }

    #[test]
    fn test_is_origin_handles_trailing_slash() {
        let whitelist = "https://example.com/";
        assert!(is_origin_allowed("https://example.com", whitelist));
    }

    #[test]
    fn test_is_origin_handles_whitespace() {
        let whitelist = " https://a.com , https://b.com ";
        assert!(is_origin_allowed("https://a.com", whitelist));
        assert!(is_origin_allowed("https://b.com", whitelist));
    }
}
