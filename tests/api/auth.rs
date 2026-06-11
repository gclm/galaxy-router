use axum::http::{Method, StatusCode};
use super::common::{app::TestApp, assert_status};

// ============================================================
// Auth — init / login / me / password
// ============================================================

#[tokio::test]
async fn test_auth_init_first_time_returns_201() {
    let app = TestApp::new_empty().await;
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/v1/init")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"username":"admin","password":"password123"}"#,
                ))
                .unwrap(),
        )
        .await;
    let body = assert_status(resp, StatusCode::CREATED).await;
    assert_eq!(body["code"], 0);
    assert!(body["data"]["token"].is_string());
}

#[tokio::test]
async fn test_auth_init_duplicate_returns_409() {
    let app = TestApp::new().await; // already has admin user
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/v1/init")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"username":"admin2","password":"password456"}"#,
                ))
                .unwrap(),
        )
        .await;
    assert_status(resp, StatusCode::CONFLICT).await;
}

#[tokio::test]
async fn test_auth_init_short_username_returns_400() {
    let app = TestApp::new_empty().await;
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/v1/init")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"username":"ab","password":"password123"}"#,
                ))
                .unwrap(),
        )
        .await;
    assert_status(resp, StatusCode::BAD_REQUEST).await;
}

#[tokio::test]
async fn test_auth_init_short_password_returns_400() {
    let app = TestApp::new_empty().await;
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/v1/init")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"username":"admin","password":"short"}"#,
                ))
                .unwrap(),
        )
        .await;
    assert_status(resp, StatusCode::BAD_REQUEST).await;
}

#[tokio::test]
async fn test_auth_login_correct_password_returns_200() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/v1/admin/auth/login")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"username":"admin","password":"password123"}"#,
                ))
                .unwrap(),
        )
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["code"], 0);
    assert!(body["data"]["token"].is_string());
}

#[tokio::test]
async fn test_auth_login_wrong_password_returns_401() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/v1/admin/auth/login")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"username":"admin","password":"WRONG"}"#,
                ))
                .unwrap(),
        )
        .await;
    assert_status(resp, StatusCode::UNAUTHORIZED).await;
}

#[tokio::test]
async fn test_auth_login_nonexistent_user_returns_401() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/v1/admin/auth/login")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"username":"nobody","password":"password123"}"#,
                ))
                .unwrap(),
        )
        .await;
    assert_status(resp, StatusCode::UNAUTHORIZED).await;
}

#[tokio::test]
async fn test_auth_me_valid_jwt_returns_200() {
    let app = TestApp::new().await;
    let resp = app.oneshot(app.admin_req(Method::GET, "/api/v1/admin/auth/me")).await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["data"]["username"], "admin");
    assert!(body["data"]["id"].is_string());
}

#[tokio::test]
async fn test_auth_me_no_jwt_returns_401() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.anon_req(Method::GET, "/api/v1/admin/auth/me"))
        .await;
    assert_status(resp, StatusCode::UNAUTHORIZED).await;
}

#[tokio::test]
async fn test_auth_me_invalid_jwt_returns_401() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/api/v1/admin/auth/me")
                .header("authorization", "Bearer invalid.jwt.token")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await;
    assert_status(resp, StatusCode::UNAUTHORIZED).await;
}

#[tokio::test]
async fn test_auth_change_password_correct_old_returns_200() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_json(
            Method::PUT,
            "/api/v1/admin/auth/password",
            r#"{"old_password":"password123","new_password":"newpassword456"}"#,
        ))
        .await;
    assert_status(resp, StatusCode::OK).await;
}

#[tokio::test]
async fn test_auth_change_password_wrong_old_returns_401() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_json(
            Method::PUT,
            "/api/v1/admin/auth/password",
            r#"{"old_password":"WRONG","new_password":"newpassword456"}"#,
        ))
        .await;
    assert_status(resp, StatusCode::UNAUTHORIZED).await;
}

#[tokio::test]
async fn test_auth_change_password_short_new_returns_400() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_json(
            Method::PUT,
            "/api/v1/admin/auth/password",
            r#"{"old_password":"password123","new_password":"short"}"#,
        ))
        .await;
    assert_status(resp, StatusCode::BAD_REQUEST).await;
}
