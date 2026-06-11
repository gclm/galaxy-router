use axum::http::{Method, StatusCode};
use super::common::{app::TestApp, assert_status};

// ============================================================
// Models List — GET /v1/models
// ============================================================

#[tokio::test]
async fn test_proxy_models_list_happy_path() {
    let app = TestApp::new_with_fixtures("http://127.0.0.1:1").await;

    let resp = app
        .oneshot(app.proxy_req(
            Method::GET,
            "/v1/models",
            "",
            app.api_key(),
        ))
        .await;
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["object"], "list");
    let models = body["data"].as_array().unwrap();
    // fixtures 创建了 6 个分组
    assert!(!models.is_empty(), "should have at least one model");
    // 每个模型对象应有 id + object 字段
    assert_eq!(models[0]["object"], "model");
}

#[tokio::test]
async fn test_proxy_models_list_no_api_key_returns_401() {
    let app = TestApp::new_empty().await;
    let resp = app
        .oneshot(app.anon_req(Method::GET, "/v1/models"))
        .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
