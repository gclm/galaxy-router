use axum::http::{Method, StatusCode};
use super::common::{app::TestApp, assert_status, mock};

// ============================================================
// Anthropic Messages — /v1/messages
// ============================================================

fn anthropic_response() -> serde_json::Value {
    serde_json::json!({
        "id": "msg-001",
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": "Hello from Claude!"}],
        "model": "claude-sonnet-4",
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 10, "output_tokens": 5}
    })
}

#[tokio::test]
async fn test_proxy_messages_happy_path() {
    let server = mock::spawn_anthropic_messages_mock(anthropic_response()).await;
    let app = TestApp::new_with_fixtures(&server.uri()).await;

    let resp = app
        .oneshot(app.proxy_req(
            Method::POST,
            "/v1/messages",
            r#"{"model":"claude-sonnet-4","max_tokens":100,"messages":[{"role":"user","content":"hi"}]}"#,
            app.api_key(),
        ))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["type"], "message");
    assert_eq!(body["content"][0]["text"], "Hello from Claude!");
}

#[tokio::test]
async fn test_proxy_messages_x_api_key_auth() {
    let server = mock::spawn_anthropic_messages_mock(anthropic_response()).await;
    let app = TestApp::new_with_fixtures(&server.uri()).await;

    // 使用 x-api-key header 而非 Authorization
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("x-api-key", app.api_key())
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"model":"claude-sonnet-4","max_tokens":100,"messages":[{"role":"user","content":"hi"}]}"#,
                ))
                .unwrap(),
        )
        .await;
    assert_status(resp, StatusCode::OK).await;
}

#[tokio::test]
async fn test_proxy_messages_anthropic_version_header() {
    let server = mock::spawn_anthropic_messages_mock(anthropic_response()).await;
    let app = TestApp::new_with_fixtures(&server.uri()).await;

    // 带 anthropic-version header
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("authorization", format!("Bearer {}", app.api_key()))
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"model":"claude-sonnet-4","max_tokens":100,"messages":[{"role":"user","content":"hi"}]}"#,
                ))
                .unwrap(),
        )
        .await;
    assert_status(resp, StatusCode::OK).await;
}

#[tokio::test]
async fn test_proxy_messages_no_auth_returns_401() {
    let app = TestApp::new_empty().await;
    let resp = app
        .oneshot(app.anon_req(Method::POST, "/v1/messages"))
        .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
