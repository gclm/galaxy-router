use axum::http::{Method, StatusCode};
use super::common::{app::TestApp, assert_status, to_json};

// ============================================================
// Models Info — list / get / update
// ============================================================

#[tokio::test]
async fn test_models_info_list_empty() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_req(Method::GET, "/api/v1/admin/models/info"))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["code"], 0);
    assert!(body["data"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_models_info_get_not_found_returns_404() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_req(
            Method::GET,
            "/api/v1/admin/models/info/nonexistent-model",
        ))
        .await;
    assert_status(resp, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn test_models_info_update_creates_or_updates() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_json(
            Method::PUT,
            "/api/v1/admin/models/info",
            r#"{"model":"gpt-4o-test","provider":"openai","input_price":5.0,"output_price":15.0}"#,
        ))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["code"], 0);
    assert_eq!(body["data"]["model"], "gpt-4o-test");
    assert_eq!(body["data"]["provider"], "openai");
    assert_eq!(body["data"]["input_price"], 5.0);
}
