use axum::http::{Method, StatusCode};
use super::common::{app::TestApp, assert_status};

// ============================================================
// Channels — CRUD + validation
// ============================================================

/// 辅助：通过 admin API 创建一个渠道，返回 response body
async fn create_channel_via_api(app: &TestApp, body: &str) -> serde_json::Value {
    let resp = app
        .oneshot(app.admin_json(Method::POST, "/api/v1/admin/channels", body))
        .await;
    assert_status(resp, StatusCode::CREATED).await
}

#[tokio::test]
async fn test_channels_create_full_fields() {
    let app = TestApp::new().await;
    let body = create_channel_via_api(
        &app,
        r#"{
            "name": "test-channel",
            "api_keys": [{"key": "sk-test-123"}],
            "endpoints": [{"base_url": "https://api.openai.com", "type": "openai_chat"}],
            "models": ["gpt-4o", "gpt-4o-mini"],
            "enabled": true
        }"#,
    )
    .await;
    assert_eq!(body["code"], 0);
    assert!(body["data"]["id"].is_string());
    assert_eq!(body["data"]["name"], "test-channel");
}

#[tokio::test]
async fn test_channels_create_minimal_fields() {
    let app = TestApp::new().await;
    let body = create_channel_via_api(
        &app,
        r#"{
            "name": "minimal-ch",
            "api_keys": [{"key": "sk-min"}],
            "endpoints": [{"base_url": "https://api.example.com", "type": "openai_chat"}]
        }"#,
    )
    .await;
    assert_eq!(body["code"], 0);
}

#[tokio::test]
async fn test_channels_create_empty_name_returns_400() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_json(
            Method::POST,
            "/api/v1/admin/channels",
            r#"{"name":"","api_keys":[{"key":"sk-1"}],"endpoints":[{"base_url":"https://a.com","type":"openai_chat"}]}"#,
        ))
        .await;
    assert_status(resp, StatusCode::BAD_REQUEST).await;
}

#[tokio::test]
async fn test_channels_create_empty_api_keys_returns_400() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_json(
            Method::POST,
            "/api/v1/admin/channels",
            r#"{"name":"ch","api_keys":[],"endpoints":[{"base_url":"https://a.com","type":"openai_chat"}]}"#,
        ))
        .await;
    assert_status(resp, StatusCode::BAD_REQUEST).await;
}

#[tokio::test]
async fn test_channels_create_empty_endpoints_returns_400() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_json(
            Method::POST,
            "/api/v1/admin/channels",
            r#"{"name":"ch","api_keys":[{"key":"sk-1"}],"endpoints":[]}"#,
        ))
        .await;
    assert_status(resp, StatusCode::BAD_REQUEST).await;
}

#[tokio::test]
async fn test_channels_create_duplicate_name_returns_409() {
    let app = TestApp::new().await;
    let body = r#"{"name":"dup-ch","api_keys":[{"key":"sk-1"}],"endpoints":[{"base_url":"https://a.com","type":"openai_chat"}]}"#;
    create_channel_via_api(&app, body).await;
    let resp = app
        .oneshot(app.admin_json(Method::POST, "/api/v1/admin/channels", body))
        .await;
    assert_status(resp, StatusCode::CONFLICT).await;
}

#[tokio::test]
async fn test_channels_list_empty() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_req(Method::GET, "/api/v1/admin/channels"))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["data"]["items"].as_array().unwrap().len(), 0);
    assert_eq!(body["data"]["total"], 0);
}

#[tokio::test]
async fn test_channels_list_with_data_and_pagination() {
    let app = TestApp::new().await;
    // 创建 3 个渠道
    for i in 1..=3 {
        create_channel_via_api(
            &app,
            &format!(
                r#"{{"name":"ch-{i}","api_keys":[{{"key":"sk-{i}"}}],"endpoints":[{{"base_url":"https://a{i}.com","type":"openai_chat"}}]}}"#,
            ),
        )
        .await;
    }
    // 第 1 页，每页 2 条
    let resp = app
        .oneshot(app.admin_req(Method::GET, "/api/v1/admin/channels?page=1&page_size=2"))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["data"]["items"].as_array().unwrap().len(), 2);
    assert_eq!(body["data"]["total"], 3);
}

#[tokio::test]
async fn test_channels_list_search() {
    let app = TestApp::new().await;
    create_channel_via_api(
        &app,
        r#"{"name":"openai-prod","api_keys":[{"key":"sk-1"}],"endpoints":[{"base_url":"https://a.com","type":"openai_chat"}]}"#,
    )
    .await;
    create_channel_via_api(
        &app,
        r#"{"name":"anthropic-prod","api_keys":[{"key":"sk-2"}],"endpoints":[{"base_url":"https://b.com","type":"openai_chat"}]}"#,
    )
    .await;
    let resp = app
        .oneshot(app.admin_req(Method::GET, "/api/v1/admin/channels?search=openai"))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["data"]["total"], 1);
}

#[tokio::test]
async fn test_channels_list_filter_by_status() {
    let app = TestApp::new().await;
    create_channel_via_api(
        &app,
        r#"{"name":"ch-enabled","api_keys":[{"key":"sk-1"}],"endpoints":[{"base_url":"https://a.com","type":"openai_chat"}],"enabled":true}"#,
    )
    .await;
    let resp = app
        .oneshot(app.admin_req(Method::GET, "/api/v1/admin/channels?status=enabled"))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["data"]["total"], 1);

    let resp = app
        .oneshot(app.admin_req(Method::GET, "/api/v1/admin/channels?status=disabled"))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["data"]["total"], 0);
}

#[tokio::test]
async fn test_channels_get_by_id() {
    let app = TestApp::new().await;
    let body = create_channel_via_api(
        &app,
        r#"{"name":"ch-get","api_keys":[{"key":"sk-1"}],"endpoints":[{"base_url":"https://a.com","type":"openai_chat"}]}"#,
    )
    .await;
    let id = body["data"]["id"].as_str().unwrap();

    let resp = app
        .oneshot(app.admin_req(Method::GET, &format!("/api/v1/admin/channels/{id}")))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["data"]["name"], "ch-get");
}

#[tokio::test]
async fn test_channels_get_not_found_returns_404() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_req(
            Method::GET,
            "/api/v1/admin/channels/00000000-0000-0000-0000-000000000000",
        ))
        .await;
    assert_status(resp, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn test_channels_update_name() {
    let app = TestApp::new().await;
    let body = create_channel_via_api(
        &app,
        r#"{"name":"before","api_keys":[{"key":"sk-1"}],"endpoints":[{"base_url":"https://a.com","type":"openai_chat"}]}"#,
    )
    .await;
    let id = body["data"]["id"].as_str().unwrap();

    let resp = app
        .oneshot(app.admin_json(
            Method::PUT,
            &format!("/api/v1/admin/channels/{id}"),
            r#"{"name":"after"}"#,
        ))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["data"]["name"], "after");
}

#[tokio::test]
async fn test_channels_update_models() {
    let app = TestApp::new().await;
    let body = create_channel_via_api(
        &app,
        r#"{"name":"ch-models","api_keys":[{"key":"sk-1"}],"endpoints":[{"base_url":"https://a.com","type":"openai_chat"}]}"#,
    )
    .await;
    let id = body["data"]["id"].as_str().unwrap();

    let resp = app
        .oneshot(app.admin_json(
            Method::PUT,
            &format!("/api/v1/admin/channels/{id}"),
            r#"{"models":["gpt-4o","claude-3-5-sonnet"]}"#,
        ))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["data"]["models"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn test_channels_update_disable() {
    let app = TestApp::new().await;
    let body = create_channel_via_api(
        &app,
        r#"{"name":"ch-disable","api_keys":[{"key":"sk-1"}],"endpoints":[{"base_url":"https://a.com","type":"openai_chat"}]}"#,
    )
    .await;
    let id = body["data"]["id"].as_str().unwrap();

    let resp = app
        .oneshot(app.admin_json(
            Method::PUT,
            &format!("/api/v1/admin/channels/{id}"),
            r#"{"enabled":false}"#,
        ))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["data"]["enabled"], false);
}

#[tokio::test]
async fn test_channels_update_not_found_returns_404() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_json(
            Method::PUT,
            "/api/v1/admin/channels/00000000-0000-0000-0000-000000000000",
            r#"{"name":"x"}"#,
        ))
        .await;
    assert_status(resp, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn test_channels_update_no_fields_returns_400() {
    let app = TestApp::new().await;
    let body = create_channel_via_api(
        &app,
        r#"{"name":"ch-noop","api_keys":[{"key":"sk-1"}],"endpoints":[{"base_url":"https://a.com","type":"openai_chat"}]}"#,
    )
    .await;
    let id = body["data"]["id"].as_str().unwrap();

    let resp = app
        .oneshot(app.admin_json(
            Method::PUT,
            &format!("/api/v1/admin/channels/{id}"),
            "{}",
        ))
        .await;
    assert_status(resp, StatusCode::BAD_REQUEST).await;
}

#[tokio::test]
async fn test_channels_delete_existing() {
    let app = TestApp::new().await;
    let body = create_channel_via_api(
        &app,
        r#"{"name":"ch-del","api_keys":[{"key":"sk-1"}],"endpoints":[{"base_url":"https://a.com","type":"openai_chat"}]}"#,
    )
    .await;
    let id = body["data"]["id"].as_str().unwrap();

    let resp = app
        .oneshot(app.admin_req(
            Method::DELETE,
            &format!("/api/v1/admin/channels/{id}"),
        ))
        .await;
    assert_status(resp, StatusCode::OK).await;
}

#[tokio::test]
async fn test_channels_delete_not_found_returns_404() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_req(
            Method::DELETE,
            "/api/v1/admin/channels/00000000-0000-0000-0000-000000000000",
        ))
        .await;
    assert_status(resp, StatusCode::NOT_FOUND).await;
}
