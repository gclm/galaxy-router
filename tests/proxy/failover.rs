use axum::http::{Method, StatusCode};
use super::common::{app::TestApp, assert_status, mock};

// ============================================================
// Failover — 渠道故障转移
// ============================================================

fn openai_chat_response() -> serde_json::Value {
    serde_json::json!({
        "id": "chatcmpl-001",
        "object": "chat.completion",
        "model": "gpt-4o",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "OK"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7}
    })
}

#[tokio::test]
async fn test_proxy_failover_primary_fails_backup_takes_over() {
    // 主渠道 mock 返回 500
    let primary = mock::spawn_error_mock(500).await;

    // 备渠道 mock 返回成功
    let backup = mock::spawn_openai_chat_mock(openai_chat_response()).await;

    let app = TestApp::new().await;
    let (_, key_str) = app.insert_api_key("failover-key", true).await;

    // 主渠道 + 分组
    let ch_primary = app
        .insert_channel_with_models(
            "primary",
            &primary.uri(),
            r#"["gpt-4o"]"#,
        )
        .await;
    app.insert_route("gpt-4o", &ch_primary, "gpt-4o").await;

    // 备渠道 + 分组
    let ch_backup = app
        .insert_channel_with_models(
            "backup",
            &backup.uri(),
            r#"["gpt-4o"]"#,
        )
        .await;
    // 添加备渠道到同一分组
    let group = sqlx::query_scalar::<_, String>("SELECT id FROM routes WHERE name = 'gpt-4o'")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let item_id = uuid::Uuid::now_v7().to_string();
    sqlx::query(
        "INSERT INTO route_items (id, route_id, channel_id, model_name, priority, weight) VALUES (?, ?, ?, ?, 1, 100)",
    )
    .bind(&item_id)
    .bind(&group)
    .bind(&ch_backup)
    .bind("gpt-4o")
    .execute(&app.pool)
    .await
    .unwrap();

    let resp = app
        .oneshot(
            app.proxy_req(
                Method::POST,
                "/v1/chat/completions",
                r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#,
                &key_str,
            ),
        )
        .await;
    // 主渠道失败后，备渠道接管
    let body = assert_status(resp, StatusCode::OK).await;
    assert_eq!(body["object"], "chat.completion");
}

#[tokio::test]
async fn test_proxy_failover_all_channels_fail_returns_error() {
    // 所有渠道都返回 500
    let server = mock::spawn_error_mock(500).await;
    let app = TestApp::new_with_fixtures(&server.uri()).await;

    let resp = app
        .oneshot(app.proxy_req(
            Method::POST,
            "/v1/chat/completions",
            r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#,
            app.api_key(),
        ))
        .await;
    // 所有渠道都失败 → 502 或 500
    let status = resp.status();
    assert!(
        status == StatusCode::INTERNAL_SERVER_ERROR
            || status == StatusCode::BAD_GATEWAY
            || status == StatusCode::SERVICE_UNAVAILABLE,
        "expected 5xx error, got {status}"
    );
}
