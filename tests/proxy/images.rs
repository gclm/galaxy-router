use axum::http::{Method, StatusCode};
use super::common::{app::TestApp, assert_status, mock};

// ============================================================
// Images — /v1/images/generations
// ============================================================

#[tokio::test]
async fn test_proxy_images_happy_path() {
    let server = mock::spawn_openai_images_mock().await;
    let app = TestApp::new_with_fixtures(&server.uri()).await;

    let resp = app
        .oneshot(app.proxy_req(
            Method::POST,
            "/v1/images/generations",
            r#"{"model":"dall-e-3","prompt":"a cat"}"#,
            app.api_key(),
        ))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert!(body["data"].is_array());
}

#[tokio::test]
async fn test_proxy_images_no_api_key_returns_401() {
    let app = TestApp::new_empty().await;
    let resp = app
        .oneshot(app.anon_req(Method::POST, "/v1/images/generations"))
        .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
