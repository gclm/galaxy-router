use axum::http::{Method, StatusCode};
use super::common::{app::TestApp, assert_status, mock};

// ============================================================
// Chat Completions — /v1/chat/completions
// ============================================================

fn openai_chat_response() -> serde_json::Value {
    serde_json::json!({
        "id": "chatcmpl-001",
        "object": "chat.completion",
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "Hello!"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
    })
}

#[tokio::test]
async fn test_proxy_chat_happy_path() {
    let server = mock::spawn_openai_chat_mock(openai_chat_response()).await;
    let app = TestApp::new_with_fixtures(&server.uri()).await;

    let resp = app
        .oneshot(app.proxy_req(
            Method::POST,
            "/v1/chat/completions",
            r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#,
            app.api_key(),
        ))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["object"], "chat.completion");
    assert_eq!(body["choices"][0]["message"]["content"], "Hello!");
}

#[tokio::test]
async fn test_proxy_chat_no_api_key_returns_401() {
    let app = TestApp::new_empty().await;
    let resp = app
        .oneshot(app.anon_req(Method::POST, "/v1/chat/completions"))
        .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_proxy_chat_invalid_api_key_returns_401() {
    let app = TestApp::new_empty().await;
    let resp = app
        .oneshot(
            app.proxy_req(
                Method::POST,
                "/v1/chat/completions",
                r#"{"model":"gpt-4o","messages":[]}"#,
                "sk-gr-nonexistent-key",
            ),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_proxy_chat_disabled_key_returns_403() {
    let app = TestApp::new().await;
    let (_, key_str) = app.insert_api_key("disabled", false).await;
    let resp = app
        .oneshot(
            app.proxy_req(
                Method::POST,
                "/v1/chat/completions",
                r#"{"model":"gpt-4o","messages":[]}"#,
                &key_str,
            ),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_proxy_chat_model_not_found_returns_404() {
    let app = TestApp::new().await;
    let (_, key_str) = app.insert_api_key("test", true).await;
    let resp = app
        .oneshot(
            app.proxy_req(
                Method::POST,
                "/v1/chat/completions",
                r#"{"model":"nonexistent-model","messages":[{"role":"user","content":"hi"}]}"#,
                &key_str,
            ),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_proxy_chat_upstream_error_passthrough() {
    let server = mock::spawn_error_mock(500).await;
    let app = TestApp::new_with_fixtures(&server.uri()).await;

    let resp = app
        .oneshot(app.proxy_req(
            Method::POST,
            "/v1/chat/completions",
            r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#,
            app.api_key(),
        ))
        .await;
    // 上游 500 应透传
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
}
