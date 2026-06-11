use axum::http::{Method, StatusCode};
use super::common::{app::TestApp, assert_status};

// ============================================================
// Groups — CRUD + items
// ============================================================

/// 辅助：创建渠道并返回 channel_id
async fn setup_channel(app: &TestApp) -> String {
    app.insert_channel("test-ch", "https://api.openai.com").await
}

/// 辅助：通过 API 创建分组
async fn create_group_via_api(app: &TestApp, body: &str) -> serde_json::Value {
    let resp = app
        .oneshot(app.admin_json(Method::POST, "/api/v1/admin/groups", body))
        .await;
    assert_status(resp, StatusCode::CREATED).await
}

#[tokio::test]
async fn test_groups_create_with_items() {
    let app = TestApp::new().await;
    let ch_id = setup_channel(&app).await;
    let body = create_group_via_api(
        &app,
        &format!(
            r#"{{"name":"gpt-4o","items":[{{"channel_id":"{ch_id}","model_name":"gpt-4o"}}]}}"#,
        ),
    )
    .await;
    assert_eq!(body["code"], 0);
    assert_eq!(body["data"]["name"], "gpt-4o");
    assert_eq!(body["data"]["items"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn test_groups_create_empty_name_returns_400() {
    let app = TestApp::new().await;
    let ch_id = setup_channel(&app).await;
    let resp = app
        .oneshot(app.admin_json(
            Method::POST,
            "/api/v1/admin/groups",
            &format!(r#"{{"name":"","items":[{{"channel_id":"{ch_id}","model_name":"gpt-4o"}}]}}"#),
        ))
        .await;
    assert_status(resp, StatusCode::BAD_REQUEST).await;
}

#[tokio::test]
async fn test_groups_create_empty_items_returns_400() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_json(
            Method::POST,
            "/api/v1/admin/groups",
            r#"{"name":"test","items":[]}"#,
        ))
        .await;
    assert_status(resp, StatusCode::BAD_REQUEST).await;
}

#[tokio::test]
async fn test_groups_create_duplicate_name_returns_409() {
    let app = TestApp::new().await;
    let ch_id = setup_channel(&app).await;
    let body = format!(r#"{{"name":"gpt-4o","items":[{{"channel_id":"{ch_id}","model_name":"gpt-4o"}}]}}"#);
    create_group_via_api(&app, &body).await;
    let resp = app
        .oneshot(app.admin_json(Method::POST, "/api/v1/admin/groups", &body))
        .await;
    assert_status(resp, StatusCode::CONFLICT).await;
}

#[tokio::test]
async fn test_groups_list_empty() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_req(Method::GET, "/api/v1/admin/groups"))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["data"]["items"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_groups_list_with_pagination() {
    let app = TestApp::new().await;
    let ch_id = setup_channel(&app).await;
    for i in 1..=3 {
        create_group_via_api(
            &app,
            &format!(
                r#"{{"name":"model-{i}","items":[{{"channel_id":"{ch_id}","model_name":"model-{i}"}}]}}"#,
            ),
        )
        .await;
    }
    let resp = app
        .oneshot(app.admin_req(Method::GET, "/api/v1/admin/groups?page=1&page_size=2"))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["data"]["items"].as_array().unwrap().len(), 2);
    assert_eq!(body["data"]["total"], 3);
}

#[tokio::test]
async fn test_groups_get_by_id() {
    let app = TestApp::new().await;
    let ch_id = setup_channel(&app).await;
    let body = create_group_via_api(
        &app,
        &format!(
            r#"{{"name":"gpt-4o","items":[{{"channel_id":"{ch_id}","model_name":"gpt-4o"}}]}}"#,
        ),
    )
    .await;
    let gid = body["data"]["id"].as_str().unwrap();

    let resp = app
        .oneshot(app.admin_req(Method::GET, &format!("/api/v1/admin/groups/{gid}")))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["data"]["name"], "gpt-4o");
    assert_eq!(body["data"]["items"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn test_groups_get_not_found_returns_404() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_req(
            Method::GET,
            "/api/v1/admin/groups/00000000-0000-0000-0000-000000000000",
        ))
        .await;
    assert_status(resp, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn test_groups_update_name_and_items() {
    let app = TestApp::new().await;
    let ch_id = setup_channel(&app).await;
    let body = create_group_via_api(
        &app,
        &format!(
            r#"{{"name":"before","items":[{{"channel_id":"{ch_id}","model_name":"gpt-4o"}}]}}"#,
        ),
    )
    .await;
    let gid = body["data"]["id"].as_str().unwrap();

    let resp = app
        .oneshot(app.admin_json(
            Method::PUT,
            &format!("/api/v1/admin/groups/{gid}"),
            &format!(r#"{{"name":"after","items":[{{"channel_id":"{ch_id}","model_name":"gpt-4o-updated"}}]}}"#),
        ))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["data"]["name"], "after");
    assert_eq!(body["data"]["items"][0]["model_name"], "gpt-4o-updated");
}

#[tokio::test]
async fn test_groups_update_empty_items_returns_400() {
    let app = TestApp::new().await;
    let ch_id = setup_channel(&app).await;
    let body = create_group_via_api(
        &app,
        &format!(
            r#"{{"name":"before","items":[{{"channel_id":"{ch_id}","model_name":"gpt-4o"}}]}}"#,
        ),
    )
    .await;
    let gid = body["data"]["id"].as_str().unwrap();

    let resp = app
        .oneshot(app.admin_json(
            Method::PUT,
            &format!("/api/v1/admin/groups/{gid}"),
            r#"{"items":[]}"#,
        ))
        .await;
    assert_status(resp, StatusCode::BAD_REQUEST).await;
}

#[tokio::test]
async fn test_groups_update_not_found_returns_404() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_json(
            Method::PUT,
            "/api/v1/admin/groups/00000000-0000-0000-0000-000000000000",
            r#"{"name":"x"}"#,
        ))
        .await;
    assert_status(resp, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn test_groups_delete_existing() {
    let app = TestApp::new().await;
    let ch_id = setup_channel(&app).await;
    let body = create_group_via_api(
        &app,
        &format!(
            r#"{{"name":"to-delete","items":[{{"channel_id":"{ch_id}","model_name":"gpt-4o"}}]}}"#,
        ),
    )
    .await;
    let gid = body["data"]["id"].as_str().unwrap();

    let resp = app
        .oneshot(app.admin_req(Method::DELETE, &format!("/api/v1/admin/groups/{gid}")))
        .await;
    assert_status(resp, StatusCode::OK).await;
}

#[tokio::test]
async fn test_groups_delete_not_found_returns_404() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_req(
            Method::DELETE,
            "/api/v1/admin/groups/00000000-0000-0000-0000-000000000000",
        ))
        .await;
    assert_status(resp, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn test_groups_add_item() {
    let app = TestApp::new().await;
    let ch_id = setup_channel(&app).await;
    let body = create_group_via_api(
        &app,
        &format!(
            r#"{{"name":"test","items":[{{"channel_id":"{ch_id}","model_name":"gpt-4o"}}]}}"#,
        ),
    )
    .await;
    let gid = body["data"]["id"].as_str().unwrap();

    let resp = app
        .oneshot(app.admin_json(
            Method::POST,
            &format!("/api/v1/admin/groups/{gid}/items"),
            &format!(r#"{{"channel_id":"{ch_id}","model_name":"gpt-4o-mini"}}"#),
        ))
        .await;
    let body = assert_status(resp, StatusCode::CREATED).await;
    assert_eq!(body["data"]["model_name"], "gpt-4o-mini");
}

#[tokio::test]
async fn test_groups_add_item_not_found_group_returns_404() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_json(
            Method::POST,
            "/api/v1/admin/groups/00000000-0000-0000-0000-000000000000/items",
            r#"{"channel_id":"x","model_name":"gpt-4o"}"#,
        ))
        .await;
    assert_status(resp, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn test_groups_delete_item() {
    let app = TestApp::new().await;
    let ch_id = setup_channel(&app).await;
    let body = create_group_via_api(
        &app,
        &format!(
            r#"{{"name":"test","items":[{{"channel_id":"{ch_id}","model_name":"gpt-4o"}}]}}"#,
        ),
    )
    .await;
    let gid = body["data"]["id"].as_str().unwrap();
    let item_id = body["data"]["items"][0]["id"].as_str().unwrap();

    let resp = app
        .oneshot(app.admin_req(
            Method::DELETE,
            &format!("/api/v1/admin/groups/{gid}/items/{item_id}"),
        ))
        .await;
    assert_status(resp, StatusCode::OK).await;
}

#[tokio::test]
async fn test_groups_delete_item_not_found_returns_404() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_req(
            Method::DELETE,
            "/api/v1/admin/groups/00000000-0000-0000-0000-000000000000/items/00000000-0000-0000-0000-000000000000",
        ))
        .await;
    assert_status(resp, StatusCode::NOT_FOUND).await;
}
