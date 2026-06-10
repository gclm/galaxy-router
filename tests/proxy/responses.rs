use axum::http::{Method, StatusCode};
use super::common::{app::TestApp, assert_status, mock};

// ============================================================
// Responses — /v1/responses
// ============================================================

#[tokio::test]
async fn test_proxy_responses_happy_path() {
    let server = mock::spawn_openai_responses_mock().await;
    let app = TestApp::new_with_fixtures(&server.uri()).await;

    let resp = app
        .oneshot(app.proxy_req(
            Method::POST,
            "/v1/responses",
            r#"{"model":"gpt-4o-responses","input":"hi"}"#,
            app.api_key(),
        ))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["object"], "response");
}

#[tokio::test]
async fn test_proxy_responses_no_api_key_returns_401() {
    let app = TestApp::new_empty().await;
    let resp = app
        .oneshot(app.anon_req(Method::POST, "/v1/responses"))
        .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
