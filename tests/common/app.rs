use axum::body::Body;
use axum::http::{Method, Request};
use galaxy_router::api::router::create_router;
use galaxy_router::auth::{JwtService, PasswordService};
use galaxy_router::config::{
    AppConfig, AuthConfig, DatabaseConfig, LoggingConfig, PricingTomlConfig, QueuingConfig,
    ServerConfig,
};
use galaxy_router::db::Database;
use galaxy_router::metrics::model::ModelRegistry;
use sqlx::SqlitePool;
use tower::ServiceExt;

/// 测试应用构建器，统一管理 DB + Router + JWT + API Key
#[allow(dead_code)]
pub struct TestApp {
    pub router: axum::Router,
    pub pool: SqlitePool,
    pub db_path: String,
    pub jwt_secret: String,
    pub admin_jwt: String,
    pub admin_user_id: String,
    /// fixtures 创建的 API Key 字符串
    api_key_str: Option<String>,
    /// fixtures 创建的 API Key ID
    api_key_id: Option<String>,
}

#[allow(dead_code)]
impl TestApp {
    /// 最小构建：空 DB + Router，无用户
    pub async fn new_empty() -> Self {
        let db_path = format!("/tmp/galaxy_test_{}.db", uuid::Uuid::now_v7());
        let _ = std::fs::remove_file(&db_path);
        let db_url = format!("sqlite:{}?mode=rwc", db_path);
        let db = Database::new(&db_url).await.unwrap();
        let pool = db.pool().clone();

        let jwt_secret = "test-secret".to_string();
        let registry = ModelRegistry::new(pool.clone());
        let config = test_config(&db_path);

        let router = create_router(
            pool.clone(),
            jwt_secret.clone(),
            &config.queuing,
            "127.0.0.1:0",
            config.clone(),
            registry,
        )
        .await;

        Self {
            router,
            pool,
            db_path,
            jwt_secret: jwt_secret.clone(),
            admin_jwt: String::new(),
            admin_user_id: String::new(),
            api_key_str: None,
            api_key_id: None,
        }
    }

    /// 标准构建：含 admin 用户（username="admin", password="password123"）
    pub async fn new() -> Self {
        let mut app = Self::new_empty().await;

        let hash = PasswordService::hash_password("password123").unwrap();
        let user_id = uuid::Uuid::now_v7().to_string();
        sqlx::query("INSERT INTO users (id, username, password_hash) VALUES (?, ?, ?)")
            .bind(&user_id)
            .bind("admin")
            .bind(&hash)
            .execute(&app.pool)
            .await
            .unwrap();

        let jwt = JwtService::new(&app.jwt_secret, 24)
            .generate_token(&user_id, "admin")
            .unwrap();

        app.admin_jwt = jwt;
        app.admin_user_id = user_id;
        app
    }

    /// 完整构建：admin + API Key + 示例渠道 + 分组
    ///
    /// 渠道的 base_url 指向 mock_url（由调用方传入 wiremock 地址），
    /// 这样代理请求会走到 mock server。
    pub async fn new_with_fixtures(mock_url: &str) -> Self {
        let mut app = Self::new().await;

        // 插入 API Key
        let (key_id, key_string) = app.insert_api_key("test-key", true).await;

        // 插入渠道：多端点 OpenAI 兼容
        let ch_id = app
            .insert_channel_multi_endpoint(
                "test-openai",
                mock_url,
                &[
                    ("openai_chat", "gpt-4o"),
                    ("openai_chat", "gpt-4o-mini"),
                    ("openai_embedding", "text-embedding-3-small"),
                    ("openai_images", "dall-e-3"),
                    ("openai_response", "gpt-4o-responses"),
                ],
            )
            .await;

        // 插入渠道：Anthropic 端点
        let ch_anthropic = app
            .insert_channel_with_endpoint(
                "test-anthropic",
                mock_url,
                "anthropic",
                r#"["claude-sonnet-4"]"#,
            )
            .await;

        // 为每个模型创建分组
        for model in &[
            "gpt-4o",
            "gpt-4o-mini",
            "text-embedding-3-small",
            "dall-e-3",
            "gpt-4o-responses",
        ] {
            app.insert_group(model, &ch_id, model).await;
        }
        app.insert_group("claude-sonnet-4", &ch_anthropic, "claude-sonnet-4")
            .await;

        app.api_key_str = Some(key_string);
        app.api_key_id = Some(key_id);
        app
    }

    // ── 请求构造 ──

    /// 发送带 admin JWT 的请求（无 body）
    pub fn admin_req(&self, method: Method, uri: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {}", self.admin_jwt))
            .body(Body::empty())
            .unwrap()
    }

    /// 发送带 admin JWT + JSON body 的请求
    pub fn admin_json(&self, method: Method, uri: &str, body: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {}", self.admin_jwt))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    /// 发送带 API Key 的代理请求
    pub fn proxy_req(&self, method: Method, uri: &str, body: &str, api_key: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {}", api_key))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    /// 发送无认证的请求
    pub fn anon_req(&self, method: Method, uri: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap()
    }

    // ── oneshot 快捷方法 ──

    /// 执行 oneshot 请求
    pub async fn oneshot(&self, req: Request<Body>) -> axum::http::Response<Body> {
        self.router.clone().oneshot(req).await.unwrap()
    }

    // ── SQL 辅助方法 ──

    /// 通过 SQL 直接插入 API Key，返回 (id, key_string)
    pub async fn insert_api_key(&self, name: &str, enabled: bool) -> (String, String) {
        let id = uuid::Uuid::now_v7().to_string();
        let key_string = format!("sk-gr-test-{}", &id[..8]);
        sqlx::query(
            "INSERT INTO api_keys (id, name, api_key, enabled) VALUES (?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(name)
        .bind(&key_string)
        .bind(enabled)
        .execute(&self.pool)
        .await
        .unwrap();
        (id, key_string)
    }

    /// 通过 SQL 直接插入渠道（OpenAI 默认端点），返回 channel_id
    pub async fn insert_channel(&self, name: &str, base_url: &str) -> String {
        let id = uuid::Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO channels (id, name, api_keys, endpoints, models, enabled) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(name)
        .bind(r#"[{"key":"sk-test-key","note":"","enabled":true}]"#)
        .bind(&format!(r#"[{{"base_url":"{}","type":"openai_chat"}}]"#, base_url))
        .bind(r#"[]"#)
        .bind(true)
        .execute(&self.pool)
        .await
        .unwrap();
        id
    }

    /// 插入渠道并指定 models
    pub async fn insert_channel_with_models(
        &self,
        name: &str,
        base_url: &str,
        models: &str,
    ) -> String {
        let id = uuid::Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO channels (id, name, api_keys, endpoints, models, enabled) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(name)
        .bind(r#"[{"key":"sk-test-key","note":"","enabled":true}]"#)
        .bind(&format!(r#"[{{"base_url":"{}","type":"openai_chat"}}]"#, base_url))
        .bind(models)
        .bind(true)
        .execute(&self.pool)
        .await
        .unwrap();
        id
    }

    /// 插入渠道并指定端点类型
    pub async fn insert_channel_with_endpoint(
        &self,
        name: &str,
        base_url: &str,
        endpoint_type: &str,
        models: &str,
    ) -> String {
        let id = uuid::Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO channels (id, name, api_keys, endpoints, models, enabled) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(name)
        .bind(r#"[{"key":"sk-test-key","note":"","enabled":true}]"#)
        .bind(&format!(r#"[{{"base_url":"{}","type":"{}"}}]"#, base_url, endpoint_type))
        .bind(models)
        .bind(true)
        .execute(&self.pool)
        .await
        .unwrap();
        id
    }

    /// 插入多端点渠道（每个 endpoint_type + model_name 对应一个端点）
    pub async fn insert_channel_multi_endpoint(
        &self,
        name: &str,
        base_url: &str,
        endpoint_model_pairs: &[(&str, &str)],
    ) -> String {
        let id = uuid::Uuid::now_v7().to_string();
        let endpoints: Vec<String> = endpoint_model_pairs
            .iter()
            .map(|(et, _)| format!(r#"{{"base_url":"{}","type":"{}"}}"#, base_url, et))
            .collect();
        let models: Vec<String> = endpoint_model_pairs
            .iter()
            .map(|(_, m)| format!(r#""{}""#, m))
            .collect();
        sqlx::query(
            "INSERT INTO channels (id, name, api_keys, endpoints, models, enabled) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(name)
        .bind(r#"[{"key":"sk-test-key","note":"","enabled":true}]"#)
        .bind(format!("[{}]", endpoints.join(",")))
        .bind(format!("[{}]", models.join(",")))
        .bind(true)
        .execute(&self.pool)
        .await
        .unwrap();
        id
    }

    /// 通过 SQL 直接插入分组 + 一个 item，返回 group_id
    pub async fn insert_group(
        &self,
        model_name: &str,
        channel_id: &str,
        target_model: &str,
    ) -> String {
        let group_id = uuid::Uuid::now_v7().to_string();
        sqlx::query("INSERT INTO groups (id, name, enabled) VALUES (?, ?, ?)")
            .bind(&group_id)
            .bind(model_name)
            .bind(true)
            .execute(&self.pool)
            .await
            .unwrap();

        let item_id = uuid::Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO group_items (id, group_id, channel_id, model_name, priority, weight) VALUES (?, ?, ?, ?, 1, 100)",
        )
        .bind(&item_id)
        .bind(&group_id)
        .bind(channel_id)
        .bind(target_model)
        .execute(&self.pool)
        .await
        .unwrap();

        group_id
    }

    // ── fixture 数据访问 ──

    /// 获取 API Key 字符串（new_with_fixtures 后可用）
    pub fn api_key(&self) -> &str {
        self.api_key_str.as_deref().unwrap_or("")
    }

    /// 获取 API Key ID（new_with_fixtures 后可用）
    pub fn api_key_id(&self) -> &str {
        self.api_key_id.as_deref().unwrap_or("")
    }
}

fn test_config(db_path: &str) -> AppConfig {
    AppConfig {
        server: ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
            timezone_offset: 0,
        },
        database: DatabaseConfig {
            path: db_path.to_string(),
        },
        logging: LoggingConfig {
            level: "warn".into(),
            format: "compact".into(),
            file: false,
            file_path: "/tmp/galaxy_test.log".into(),
            rotation: "daily".into(),
            max_files: 30,
        },
        auth: AuthConfig {
            jwt_secret: "test-secret".into(),
            token_expiry_hours: 24,
        },
        queuing: QueuingConfig::default(),
        pricing: PricingTomlConfig {
            cache_path: "/tmp/galaxy_test_pricing.json".into(),
            refresh_interval_hours: 24,
            providers: vec![],
        },
    }
}
