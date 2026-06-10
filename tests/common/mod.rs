pub mod app;
pub mod mock;

use axum::body::Body;
use axum::http::StatusCode;

/// 从 axum Response 提取 JSON body
pub async fn to_json(resp: axum::http::Response<Body>) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .expect("failed to read response body");
    if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            panic!(
                "response is not valid JSON: {e}\nbody: {}",
                String::from_utf8_lossy(&bytes)
            )
        })
    }
}

/// 断言响应状态码并返回 JSON
pub async fn assert_status(
    resp: axum::http::Response<Body>,
    expected: StatusCode,
) -> serde_json::Value {
    let status = resp.status();
    let body = to_json(resp).await;
    assert_eq!(status, expected, "body: {body}");
    body
}
