use axum::http::{Method, StatusCode};
use super::common::{app::TestApp, assert_status, to_json};

// ============================================================
// System — health / system-info
// ============================================================

#[tokio::test]
async fn test_health_uninitialized_returns_needs_setup_true() {
    let app = TestApp::new_empty().await;
    let resp = app
        .oneshot(app.anon_req(Method::GET, "/api/v1/health"))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["status"], "ok");
    assert_eq!(body["needs_setup"], true);
}

#[tokio::test]
async fn test_health_initialized_returns_needs_setup_false() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.anon_req(Method::GET, "/api/v1/health"))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["status"], "ok");
    assert_eq!(body["needs_setup"], false);
}

#[tokio::test]
async fn test_system_info_returns_200() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_req(Method::GET, "/api/v1/admin/system-info"))
        .await;
    assert_status(resp, StatusCode::OK).await;
}
