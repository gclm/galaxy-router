use axum::http::{Method, StatusCode};
use super::common::{app::TestApp, assert_status};

// ============================================================
// API Keys — CRUD + disabled key proxy
// ============================================================

/// 辅助：通过 admin API 创建 API Key
async fn create_key_via_api(app: &TestApp, body: &str) -> serde_json::Value {
    let resp = app
        .oneshot(app.admin_json(Method::POST, "/api/v1/admin/api-keys", body))
        .await;
    assert_status(resp, StatusCode::CREATED).await
}

#[tokio::test]
async fn test_api_keys_create_returns_key_starting_with_gp() {
    let app = TestApp::new().await;
    let body = create_key_via_api(&app, r#"{"name":"test-key"}"#).await;
    assert_eq!(body["code"], 0);
    let key = body["data"]["api_key"].as_str().unwrap();
    assert!(key.starts_with("sk-gr-"), "API key should start with 'sk-gr-', got: {key}");
    assert!(body["data"]["id"].is_string());
}

#[tokio::test]
async fn test_api_keys_create_empty_name_returns_400() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_json(Method::POST, "/api/v1/admin/api-keys", r#"{"name":""}"#))
        .await;
    assert_status(resp, StatusCode::BAD_REQUEST).await;
}

#[tokio::test]
async fn test_api_keys_list_empty() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_req(Method::GET, "/api/v1/admin/api-keys"))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert!(body["data"]["items"].is_array());
    assert_eq!(body["data"]["items"].as_array().unwrap().len(), 0);
    assert_eq!(body["data"]["total"], 0);
}

#[tokio::test]
async fn test_api_keys_list_with_data() {
    let app = TestApp::new().await;
    create_key_via_api(&app, r#"{"name":"key-1"}"#).await;
    create_key_via_api(&app, r#"{"name":"key-2"}"#).await;
    let resp = app
        .oneshot(app.admin_req(Method::GET, "/api/v1/admin/api-keys"))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["data"]["items"].as_array().unwrap().len(), 2);
    assert_eq!(body["data"]["total"], 2);
}

#[tokio::test]
async fn test_api_keys_list_pagination() {
    let app = TestApp::new().await;
    create_key_via_api(&app, r#"{"name":"key-a"}"#).await;
    create_key_via_api(&app, r#"{"name":"key-b"}"#).await;
    create_key_via_api(&app, r#"{"name":"key-c"}"#).await;

    let resp = app
        .oneshot(app.admin_req(Method::GET, "/api/v1/admin/api-keys?page=1&page_size=2"))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["data"]["items"].as_array().unwrap().len(), 2);
    assert_eq!(body["data"]["total"], 3);
}

#[tokio::test]
async fn test_api_keys_list_search() {
    let app = TestApp::new().await;
    create_key_via_api(&app, r#"{"name":"my-special-key"}"#).await;
    create_key_via_api(&app, r#"{"name":"other-key"}"#).await;

    let resp = app
        .oneshot(app.admin_req(Method::GET, "/api/v1/admin/api-keys?search=special"))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["data"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(body["data"]["total"], 1);
    assert_eq!(body["data"]["items"][0]["name"], "my-special-key");
}

#[tokio::test]
async fn test_api_keys_get_by_id() {
    let app = TestApp::new().await;
    let body = create_key_via_api(&app, r#"{"name":"get-test"}"#).await;
    let id = body["data"]["id"].as_str().unwrap();

    let resp = app
        .oneshot(app.admin_req(Method::GET, &format!("/api/v1/admin/api-keys/{id}")))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["data"]["name"], "get-test");
}

#[tokio::test]
async fn test_api_keys_get_not_found_returns_404() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_req(
            Method::GET,
            "/api/v1/admin/api-keys/00000000-0000-0000-0000-000000000000",
        ))
        .await;
    assert_status(resp, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn test_api_keys_update_name() {
    let app = TestApp::new().await;
    let body = create_key_via_api(&app, r#"{"name":"before"}"#).await;
    let id = body["data"]["id"].as_str().unwrap();

    let resp = app
        .oneshot(app.admin_json(
            Method::PUT,
            &format!("/api/v1/admin/api-keys/{id}"),
            r#"{"name":"after"}"#,
        ))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["data"]["name"], "after");
}

#[tokio::test]
async fn test_api_keys_update_disable() {
    let app = TestApp::new().await;
    let body = create_key_via_api(&app, r#"{"name":"disable-test"}"#).await;
    let id = body["data"]["id"].as_str().unwrap();

    let resp = app
        .oneshot(app.admin_json(
            Method::PUT,
            &format!("/api/v1/admin/api-keys/{id}"),
            r#"{"enabled":false}"#,
        ))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["data"]["enabled"], false);
}

#[tokio::test]
async fn test_api_keys_update_no_fields_returns_400() {
    let app = TestApp::new().await;
    let body = create_key_via_api(&app, r#"{"name":"noop-test"}"#).await;
    let id = body["data"]["id"].as_str().unwrap();

    let resp = app
        .oneshot(app.admin_json(
            Method::PUT,
            &format!("/api/v1/admin/api-keys/{id}"),
            "{}",
        ))
        .await;
    assert_status(resp, StatusCode::BAD_REQUEST).await;
}

#[tokio::test]
async fn test_api_keys_update_not_found_returns_404() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_json(
            Method::PUT,
            "/api/v1/admin/api-keys/00000000-0000-0000-0000-000000000000",
            r#"{"name":"x"}"#,
        ))
        .await;
    assert_status(resp, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn test_api_keys_delete_existing() {
    let app = TestApp::new().await;
    let body = create_key_via_api(&app, r#"{"name":"del-test"}"#).await;
    let id = body["data"]["id"].as_str().unwrap();

    let resp = app
        .oneshot(app.admin_req(Method::DELETE, &format!("/api/v1/admin/api-keys/{id}")))
        .await;
    assert_status(resp, StatusCode::OK).await;
}

#[tokio::test]
async fn test_api_keys_delete_not_found_returns_404() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_req(
            Method::DELETE,
            "/api/v1/admin/api-keys/00000000-0000-0000-0000-000000000000",
        ))
        .await;
    assert_status(resp, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn test_api_keys_disabled_key_rejected_by_proxy() {
    let app = TestApp::new().await;
    // 插入一个 disabled API Key
    let (_, key_str) = app.insert_api_key("disabled-key", false).await;

    let resp = app
        .oneshot(app.proxy_req(
            Method::POST,
            "/v1/chat/completions",
            r#"{"model":"gpt-4o","messages":[]}"#,
            &key_str,
        ))
        .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
