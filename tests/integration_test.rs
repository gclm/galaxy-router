use std::io::Write;

#[tokio::test]
async fn test_config_load() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let mut f = std::fs::File::create(&config_path).unwrap();
    write!(
        f,
        r#"
[server]
host = "0.0.0.0"
port = 9090

[database]
path = "test/galaxy.db"

[auth]
jwt_secret = "test-secret-for-ci"
token_expiry_hours = 24

[logging]
level = "debug"
format = "compact"
file = false
file_path = "test/galaxy.log"

[pricing]
cache_path = "test/pricing_cache.json"
refresh_interval_hours = 24
providers = ["openai"]
"#
    )
    .unwrap();
    drop(f);

    let config = galaxy_router::config::AppConfig::load(&config_path).unwrap();

    assert_eq!(config.server.host, "0.0.0.0");
    assert_eq!(config.server.port, 9090);
    assert_eq!(config.database.path, "test/galaxy.db");
}

#[tokio::test]
async fn test_database_init() {
    let db_path = "/tmp/galaxy_test/test.db";
    let db_url = format!("sqlite:{}?mode=rwc", db_path);

    let _ = std::fs::remove_file(db_path);
    let _ = std::fs::remove_dir_all("/tmp/galaxy_test");
    std::fs::create_dir_all("/tmp/galaxy_test").unwrap();

    let db = galaxy_router::db::Database::new(&db_url).await.unwrap();

    let tables: Vec<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .fetch_all(db.pool())
            .await
            .unwrap();

    assert!(tables.contains(&"users".to_string()));
    assert!(tables.contains(&"channels".to_string()));
    assert!(tables.contains(&"groups".to_string()));
    assert!(tables.contains(&"api_keys".to_string()));
    assert!(tables.contains(&"usage_logs".to_string()));
    assert!(tables.contains(&"usage_daily".to_string()));
    assert!(tables.contains(&"settings".to_string()));

    let _ = std::fs::remove_file(db_path);
    let _ = std::fs::remove_dir_all("/tmp/galaxy_test");
}

#[test]
fn test_password_hash() {
    let password = "test_password_123";
    let hash = galaxy_router::auth::PasswordService::hash_password(password).unwrap();

    assert!(galaxy_router::auth::PasswordService::verify_password(password, &hash).unwrap());
    assert!(!galaxy_router::auth::PasswordService::verify_password("wrong", &hash).unwrap());
}

#[test]
fn test_jwt_token() {
    let jwt_service = galaxy_router::auth::JwtService::new("test_secret", 24);
    let token = jwt_service.generate_token("1", "admin").unwrap();
    let claims = galaxy_router::auth::decode_jwt(&token, "test_secret").unwrap();

    assert_eq!(claims.sub, "1");
    assert_eq!(claims.username, "admin");
}

// ============================================================
// 渠道多端点测试
// ============================================================

#[tokio::test]
async fn test_channel_multi_endpoint() {
    let db_path = "/tmp/galaxy_test_channel_multi/test.db";
    let db_url = format!("sqlite:{}?mode=rwc", db_path);

    let _ = std::fs::remove_dir_all("/tmp/galaxy_test_channel_multi");
    std::fs::create_dir_all("/tmp/galaxy_test_channel_multi").unwrap();

    let db = galaxy_router::db::Database::new(&db_url).await.unwrap();
    let pool = db.pool().clone();

    // 创建多端点渠道（类似百炼 Coding Plan）
    let channel_id = uuid::Uuid::now_v7().to_string();
    let endpoints = r#"[
        {"type": "openai_chat", "base_url": "https://coding.dashscope.aliyuncs.com/v1"},
        {"type": "anthropic", "base_url": "https://coding.dashscope.aliyuncs.com/apps/anthropic/v1"}
    ]"#;

    sqlx::query("INSERT INTO channels (id, name, api_keys, endpoints) VALUES (?, ?, ?, ?)")
        .bind(&channel_id)
        .bind("百炼 Coding Plan")
        .bind(r#"["sk-test-key"]"#)
        .bind(endpoints)
        .execute(&pool)
        .await
        .unwrap();

    // 查询渠道
    let (name, api_keys, endpoints_str): (String, String, String) =
        sqlx::query_as("SELECT name, api_keys, endpoints FROM channels WHERE id = ?")
            .bind(&channel_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(name, "百炼 Coding Plan");
    assert_eq!(api_keys, r#"["sk-test-key"]"#);

    // 验证 endpoints JSON 解析
    let parsed_endpoints: Vec<serde_json::Value> = serde_json::from_str(&endpoints_str).unwrap();
    assert_eq!(parsed_endpoints.len(), 2);
    assert_eq!(parsed_endpoints[0]["type"], "openai_chat");
    assert_eq!(parsed_endpoints[1]["type"], "anthropic");

    let _ = std::fs::remove_dir_all("/tmp/galaxy_test_channel_multi");
}

#[tokio::test]
async fn test_channel_single_endpoint() {
    let db_path = "/tmp/galaxy_test_channel_single/test.db";
    let db_url = format!("sqlite:{}?mode=rwc", db_path);

    let _ = std::fs::remove_dir_all("/tmp/galaxy_test_channel_single");
    std::fs::create_dir_all("/tmp/galaxy_test_channel_single").unwrap();

    let db = galaxy_router::db::Database::new(&db_url).await.unwrap();
    let pool = db.pool().clone();

    // 创建单端点渠道
    let channel_id = uuid::Uuid::now_v7().to_string();
    let endpoints = r#"[{"type": "openai_chat", "base_url": "https://api.openai.com/v1"}]"#;

    sqlx::query("INSERT INTO channels (id, name, api_keys, endpoints) VALUES (?, ?, ?, ?)")
        .bind(&channel_id)
        .bind("OpenAI Official")
        .bind(r#"["sk-xxx"]"#)
        .bind(endpoints)
        .execute(&pool)
        .await
        .unwrap();

    let endpoints_str: String = sqlx::query_scalar("SELECT endpoints FROM channels WHERE id = ?")
        .bind(&channel_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    let parsed: Vec<serde_json::Value> = serde_json::from_str(&endpoints_str).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0]["type"], "openai_chat");

    let _ = std::fs::remove_dir_all("/tmp/galaxy_test_channel_single");
}

// ============================================================
// 分组测试
// ============================================================

#[tokio::test]
async fn test_group_with_channel() {
    let db_path = "/tmp/galaxy_test_group_channel/test.db";
    let db_url = format!("sqlite:{}?mode=rwc", db_path);

    let _ = std::fs::remove_dir_all("/tmp/galaxy_test_group_channel");
    std::fs::create_dir_all("/tmp/galaxy_test_group_channel").unwrap();

    let db = galaxy_router::db::Database::new(&db_url).await.unwrap();
    let pool = db.pool().clone();

    // 创建渠道
    let channel_id = uuid::Uuid::now_v7().to_string();
    sqlx::query("INSERT INTO channels (id, name, api_keys, endpoints) VALUES (?, ?, ?, ?)")
        .bind(&channel_id)
        .bind("test-channel")
        .bind(r#"["sk-test"]"#)
        .bind(r#"[{"type":"openai_chat","base_url":"https://api.openai.com/v1"}]"#)
        .execute(&pool)
        .await
        .unwrap();

    // 创建分组
    let group_id = uuid::Uuid::now_v7().to_string();
    sqlx::query("INSERT INTO groups (id, name) VALUES (?, ?)")
        .bind(&group_id)
        .bind("gpt-4o")
        .execute(&pool)
        .await
        .unwrap();

    // 添加分组项
    let item_id = uuid::Uuid::now_v7().to_string();
    sqlx::query(
        "INSERT INTO group_items (id, group_id, channel_id, model_name, priority, weight) VALUES (?, ?, ?, ?, ?, ?)"
    )
    .bind(&item_id)
    .bind(&group_id)
    .bind(&channel_id)
    .bind("gpt-4o-2024-08-06")
    .bind(1)
    .bind(100)
    .execute(&pool)
    .await
    .unwrap();

    // 验证分组和分组项
    let group_name: String = sqlx::query_scalar("SELECT name FROM groups WHERE id = ?")
        .bind(&group_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(group_name, "gpt-4o");

    let item_count: i32 = sqlx::query_scalar("SELECT COUNT(*) FROM group_items WHERE group_id = ?")
        .bind(&group_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(item_count, 1);

    let _ = std::fs::remove_dir_all("/tmp/galaxy_test_group_channel");
}

// ============================================================
// API Key 测试
// ============================================================

#[tokio::test]
async fn test_api_key_crud() {
    let db_path = "/tmp/galaxy_test_api_key/test.db";
    let db_url = format!("sqlite:{}?mode=rwc", db_path);

    let _ = std::fs::remove_dir_all("/tmp/galaxy_test_api_key");
    std::fs::create_dir_all("/tmp/galaxy_test_api_key").unwrap();

    let db = galaxy_router::db::Database::new(&db_url).await.unwrap();
    let pool = db.pool().clone();

    let key_id = uuid::Uuid::now_v7().to_string();
    let api_key = format!("gp-{}", uuid::Uuid::now_v7());

    sqlx::query("INSERT INTO api_keys (id, name, api_key, enabled) VALUES (?, ?, ?, ?)")
        .bind(&key_id)
        .bind("test-key")
        .bind(&api_key)
        .bind(true)
        .execute(&pool)
        .await
        .unwrap();

    let fetched_name: String = sqlx::query_scalar("SELECT name FROM api_keys WHERE id = ?")
        .bind(&key_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(fetched_name, "test-key");

    let enabled: bool = sqlx::query_scalar("SELECT enabled FROM api_keys WHERE api_key = ?")
        .bind(&api_key)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(enabled);

    let _ = std::fs::remove_dir_all("/tmp/galaxy_test_api_key");
}

// ============================================================
// 协议转换测试
// ============================================================

#[test]
fn test_openai_chat_transform() {
    use galaxy_router::protocol::inbound::Inbound;
    use galaxy_router::protocol::openai_chat::OpenAiChatInbound;

    let inbound = OpenAiChatInbound;
    let headers = axum::http::HeaderMap::new();

    let body = r#"{
        "model": "gpt-4o",
        "messages": [
            {"role": "user", "content": "Hello"}
        ],
        "stream": false
    }"#;

    let request =
        tokio_test::block_on(inbound.transform_request(body.as_bytes(), &headers)).unwrap();

    assert_eq!(request.model, "gpt-4o");
    assert_eq!(request.messages.len(), 1);
    assert_eq!(
        request.messages[0].role,
        galaxy_router::protocol::model::Role::User
    );
}

#[test]
fn test_anthropic_transform() {
    use galaxy_router::protocol::anthropic::AnthropicInbound;
    use galaxy_router::protocol::inbound::Inbound;

    let inbound = AnthropicInbound;
    let headers = axum::http::HeaderMap::new();

    let body = r#"{
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [
            {"role": "user", "content": "Hello"}
        ]
    }"#;

    let request =
        tokio_test::block_on(inbound.transform_request(body.as_bytes(), &headers)).unwrap();

    assert_eq!(request.model, "claude-sonnet-4-20250514");
    assert_eq!(request.messages.len(), 1);
}

// ============================================================
// 端点类型测试
// ============================================================

#[test]
fn test_endpoint_type_paths() {
    use galaxy_router::api::handlers::admin::channels::EndpointType;

    assert_eq!(EndpointType::OpenAiChat.path(), "/chat/completions");
    assert_eq!(EndpointType::OpenAiResponse.path(), "/responses");
    assert_eq!(EndpointType::Anthropic.path(), "/messages");
    assert_eq!(EndpointType::OpenAiEmbedding.path(), "/embeddings");
    assert_eq!(EndpointType::OpenAiImages.path(), "/images/generations");
}

#[test]
fn test_endpoint_type_serialization() {
    use galaxy_router::api::handlers::admin::channels::EndpointType;

    // 序列化
    let json = serde_json::to_string(&EndpointType::OpenAiChat).unwrap();
    assert_eq!(json, "\"openai_chat\"");

    let json = serde_json::to_string(&EndpointType::Anthropic).unwrap();
    assert_eq!(json, "\"anthropic\"");

    // 反序列化
    let et: EndpointType = serde_json::from_str("\"openai_chat\"").unwrap();
    assert_eq!(et, EndpointType::OpenAiChat);

    let et: EndpointType = serde_json::from_str("\"anthropic\"").unwrap();
    assert_eq!(et, EndpointType::Anthropic);
}
