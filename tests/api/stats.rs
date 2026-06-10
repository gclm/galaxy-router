use axum::http::{Method, StatusCode};
use super::common::{app::TestApp, assert_status, to_json};

// ============================================================
// Stats — overview / models / channels / daily / api-keys / latency / budgets / logs
// ============================================================

#[tokio::test]
async fn test_stats_overview_returns_200() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_req(Method::GET, "/api/v1/admin/stats/overview"))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["code"], 0);
}

#[tokio::test]
async fn test_stats_models_default_days() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_req(Method::GET, "/api/v1/admin/stats/models"))
        .await;
    assert_status(resp, StatusCode::OK).await;
}

#[tokio::test]
async fn test_stats_models_custom_date_range() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_req(Method::GET, "/api/v1/admin/stats/models?start_date=2026-01-01&end_date=2026-06-01"))
        .await;
    assert_status(resp, StatusCode::OK).await;
}

#[tokio::test]
async fn test_stats_channels_returns_200() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_req(Method::GET, "/api/v1/admin/stats/channels"))
        .await;
    assert_status(resp, StatusCode::OK).await;
}

#[tokio::test]
async fn test_stats_daily_returns_200() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_req(Method::GET, "/api/v1/admin/stats/daily"))
        .await;
    assert_status(resp, StatusCode::OK).await;
}

#[tokio::test]
async fn test_stats_api_keys_returns_200() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_req(Method::GET, "/api/v1/admin/stats/api-keys"))
        .await;
    assert_status(resp, StatusCode::OK).await;
}

#[tokio::test]
async fn test_stats_latency_returns_200() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_req(Method::GET, "/api/v1/admin/stats/latency"))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    // 空数据时返回 null 是合理的
    assert!(body["data"]["p50_latency_ms"].is_number() || body["data"]["p50_latency_ms"].is_null());
}

#[tokio::test]
async fn test_stats_logs_empty() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_req(Method::GET, "/api/v1/admin/stats/logs"))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["data"]["total"], 0);
}

#[tokio::test]
async fn test_stats_set_budget() {
    let app = TestApp::new().await;
    let (key_id, _) = app.insert_api_key("budget-key", true).await;

    let resp = app
        .oneshot(app.admin_json(
            Method::POST,
            "/api/v1/admin/budgets",
            &format!(r#"{{"api_key_id":"{key_id}","monthly_limit_usd":10.0,"daily_limit_usd":1.0}}"#),
        ))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["data"]["monthly_limit_usd"], 10.0);
    assert_eq!(body["data"]["daily_limit_usd"], 1.0);
}

#[tokio::test]
async fn test_stats_list_budgets_empty() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_req(Method::GET, "/api/v1/admin/budgets"))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["data"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_stats_delete_budget() {
    let app = TestApp::new().await;
    let (key_id, _) = app.insert_api_key("budget-del-key", true).await;

    // 先创建
    let resp = app
        .oneshot(app.admin_json(
            Method::POST,
            "/api/v1/admin/budgets",
            &format!(r#"{{"api_key_id":"{key_id}","monthly_limit_usd":5.0}}"#),
        ))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    let budget_id = body["data"]["id"].as_str().unwrap();

    // 再删除
    let resp = app
        .oneshot(app.admin_req(
            Method::DELETE,
            &format!("/api/v1/admin/budgets/{budget_id}"),
        ))
        .await;
    assert_status(resp, StatusCode::OK).await;
}

#[tokio::test]
async fn test_stats_log_detail_not_found_returns_404() {
    let app = TestApp::new().await;
    let resp = app
        .oneshot(app.admin_req(
            Method::GET,
            "/api/v1/admin/stats/logs/00000000-0000-0000-0000-000000000000",
        ))
        .await;
    assert_status(resp, StatusCode::NOT_FOUND).await;
}
