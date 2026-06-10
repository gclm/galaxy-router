use axum::http::{Method, StatusCode};
use super::common::{app::TestApp, assert_status, mock};

// ============================================================
// Embeddings — /v1/embeddings
// ============================================================

#[tokio::test]
async fn test_proxy_embeddings_happy_path() {
    let server = mock::spawn_openai_embeddings_mock().await;
    let app = TestApp::new_with_fixtures(&server.uri()).await;

    let resp = app
        .oneshot(app.proxy_req(
            Method::POST,
            "/v1/embeddings",
            r#"{"model":"text-embedding-3-small","input":"hello"}"#,
            app.api_key(),
        ))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["object"], "list");
}

#[tokio::test]
async fn test_proxy_embeddings_no_api_key_returns_401() {
    let app = TestApp::new_empty().await;
    let resp = app
        .oneshot(app.anon_req(Method::POST, "/v1/embeddings"))
        .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
