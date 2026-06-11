use axum::http::{Method, StatusCode};
use super::common::{app::TestApp, assert_status};

// ============================================================
// Backup — export / import / reset / roundtrip
// ============================================================

#[tokio::test]
async fn test_backup_export_empty_data() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_req(Method::GET, "/api/v1/admin/backups"))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["data"]["format"], "galaxy-router-backup");
    assert_eq!(body["data"]["version"], 1);
    assert!(body["data"]["exported_at"].is_string());
    assert!(body["data"]["data"]["channels"].is_array());
    assert!(body["data"]["data"]["groups"].is_array());
    assert!(body["data"]["data"]["api_keys"].is_array());
    assert!(body["data"]["data"]["settings"].is_array());
}

#[tokio::test]
async fn test_backup_export_with_data() {
    let app = TestApp::new().await;
    // 插入一些数据
    app.insert_channel("test-ch", "https://api.openai.com").await;
    app.insert_api_key("test-key", true).await;

    let resp = app
        .oneshot(app.admin_req(Method::GET, "/api/v1/admin/backups"))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["data"]["data"]["channels"].as_array().unwrap().len(), 1);
    assert_eq!(body["data"]["data"]["api_keys"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn test_backup_import_valid() {
    let app = TestApp::new().await;
    let import_body = r#"{
        "format": "galaxy-router-backup",
        "version": 1,
        "exported_at": "2026-01-01T00:00:00Z",
        "app_version": "0.0.1",
        "data": {
            "channels": [],
            "groups": [],
            "api_keys": [{"name": "imported-key", "api_key": "sk-gr-imported-001", "enabled": true}],
            "settings": [{"key": "scheduler.top_k", "value": "5"}]
        }
    }"#;
    let resp = app
        .oneshot(app.admin_json(Method::POST, "/api/v1/admin/backups", import_body))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["code"], 0);
    assert_eq!(body["data"]["api_keys_imported"], 1);
    assert_eq!(body["data"]["settings_imported"], 1);
}

#[tokio::test]
async fn test_backup_import_invalid_format_returns_400() {
    let app = TestApp::new().await;
    let import_body = r#"{
        "format": "wrong-format",
        "version": 1,
        "exported_at": "2026-01-01T00:00:00Z",
        "app_version": "0.0.1",
        "data": {"channels":[],"groups":[],"api_keys":[],"settings":[]}
    }"#;
    let resp = app
        .oneshot(app.admin_json(Method::POST, "/api/v1/admin/backups", import_body))
        .await;
    assert_status(resp, StatusCode::BAD_REQUEST).await;
}

#[tokio::test]
async fn test_backup_import_version_mismatch_returns_400() {
    let app = TestApp::new().await;
    let import_body = r#"{
        "format": "galaxy-router-backup",
        "version": 999,
        "exported_at": "2026-01-01T00:00:00Z",
        "app_version": "0.0.1",
        "data": {"channels":[],"groups":[],"api_keys":[],"settings":[]}
    }"#;
    let resp = app
        .oneshot(app.admin_json(Method::POST, "/api/v1/admin/backups", import_body))
        .await;
    assert_status(resp, StatusCode::BAD_REQUEST).await;
}

#[tokio::test]
async fn test_backup_reset_returns_counts() {
    let app = TestApp::new().await;
    app.insert_channel("ch-1", "https://api.openai.com").await;
    app.insert_api_key("key-1", true).await;

    let resp = app
        .oneshot(app.admin_req(Method::DELETE, "/api/v1/admin/backups"))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["code"], 0);
    assert!(body["data"]["channels_deleted"].as_u64().unwrap() > 0);
    assert!(body["data"]["api_keys_deleted"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn test_backup_export_reset_import_roundtrip() {
    let app = TestApp::new().await;
    // 创建数据
    app.insert_channel("roundtrip-ch", "https://api.openai.com").await;
    app.insert_api_key("roundtrip-key", true).await;

    // 1. Export
    let resp = app
        .oneshot(app.admin_req(Method::GET, "/api/v1/admin/backups"))
        .await;
    let export_body = assert_status(resp, StatusCode::OK).await;
    let exported_data = serde_json::to_string(&export_body["data"]).unwrap();

    // 2. Reset
    let resp = app
        .oneshot(app.admin_req(Method::DELETE, "/api/v1/admin/backups"))
        .await;
    assert_status(resp, StatusCode::OK).await;

    // 3. Import
    let resp = app
        .oneshot(app.admin_json(Method::POST, "/api/v1/admin/backups", &exported_data))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["data"]["channels_imported"], 1);
    assert_eq!(body["data"]["api_keys_imported"], 1);
}
