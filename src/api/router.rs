use axum::{
    Json, Router,
    body::Body,
    extract::DefaultBodyLimit,
    http::Request,
    middleware,
    routing::{delete, get, post, put},
};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use tower_http::trace::TraceLayer;

use crate::api::handlers::admin::api_keys::{self, ApiKeyState};
use crate::api::handlers::admin::auth::{self, AuthState};
use crate::api::handlers::admin::backup::{self, BackupState};
use crate::api::handlers::admin::channels::{self, ChannelState};
use crate::api::handlers::admin::fetch_models::{self, FetchModelsState};
use crate::api::handlers::admin::groups::{self, GroupState};
use crate::api::handlers::admin::model_info::{self, ModelInfoState};
use crate::api::handlers::admin::settings::{self, SettingsState};
use crate::api::handlers::admin::stats::{self, StatsApiState};
use crate::api::handlers::admin::system_info::{self, SystemInfoState};
use crate::api::handlers::proxy::{chat, embeddings, images, messages, models, responses};
use crate::api::middleware::require_admin_auth;
use crate::config::{AppConfig, QueuingConfig};
use crate::proxy::{ProxyCache, ProxyState};
use crate::static_assets;
use crate::stats::StatsState;

/// 创建应用路由
pub async fn create_router(
    pool: SqlitePool,
    jwt_secret: String,
    queuing: &QueuingConfig,
    _server_addr: &str,
    config: AppConfig,
    model_registry: crate::stats::model::ModelRegistry,
) -> Router {
    let token_expiry_hours = config.auth.token_expiry_hours;
    let auth_state = AuthState {
        pool: pool.clone(),
        jwt_service: crate::auth::JwtService::new(&jwt_secret, token_expiry_hours),
    };
    let auth_state_for_public = auth_state.clone();

    let shared_cache = ProxyCache::new();

    let tz = config.server.timezone_offset;

    let channel_state = ChannelState {
        pool: pool.clone(),
        cache: shared_cache.clone(),
        http_client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client"),
        timezone_offset: tz,
    };

    let group_state = GroupState {
        pool: pool.clone(),
        cache: shared_cache.clone(),
        timezone_offset: tz,
    };

    let api_key_cache = crate::api::middleware::ApiKeyCache::new();
    let api_key_state = ApiKeyState {
        pool: pool.clone(),
        api_key_cache,
        timezone_offset: tz,
    };

    let stats_state = StatsApiState {
        stats: StatsState::new(pool.clone(), config.server.timezone_offset),
    };

    let model_info_state = ModelInfoState {
        model_registry: model_registry.clone(),
    };

    let fetch_models_state = FetchModelsState {
        http_client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client"),
    };

    let system_info_state = SystemInfoState {
        pool: pool.clone(),
        start_time: std::sync::Arc::new(std::time::Instant::now()),
    };

    let settings_state = SettingsState {
        pool: pool.clone(),
        config: std::sync::Arc::new(config),
    };

    let backup_state = BackupState { pool: pool.clone() };

    let proxy_state = if queuing.enabled {
        ProxyState::new(pool.clone(), model_registry.clone())
            .await
            .with_queue(queuing.max_queue_size, queuing.queue_timeout_secs)
    } else {
        ProxyState::new(pool.clone(), model_registry.clone()).await
    };

    // 需要认证的管理路由（每个 nest 独立管理状态）
    let protected_admin = Router::new()
        .nest(
            "/auth",
            Router::new()
                .route("/me", get(auth::me))
                .route("/password", put(auth::change_password))
                .with_state(auth_state),
        )
        .nest(
            "/channels",
            Router::new()
                .route("/", get(channels::list).post(channels::create))
                .route(
                    "/{id}",
                    get(channels::get)
                        .put(channels::update)
                        .delete(channels::delete),
                )
                .route("/{id}/test", post(channels::test_channel))
                .route("/{id}/detect", post(channels::detect_channel_quirks))
                .with_state(channel_state),
        )
        .nest(
            "/groups",
            Router::new()
                .route("/", get(groups::list).post(groups::create))
                .route(
                    "/{id}",
                    get(groups::get).put(groups::update).delete(groups::delete),
                )
                .route("/{id}/items", post(groups::add_item))
                .route("/{id}/items/{item_id}", delete(groups::delete_item))
                .with_state(group_state),
        )
        .nest(
            "/api-keys",
            Router::new()
                .route("/", get(api_keys::list).post(api_keys::create))
                .route(
                    "/{id}",
                    get(api_keys::get)
                        .put(api_keys::update)
                        .delete(api_keys::delete),
                )
                .with_state(api_key_state),
        )
        .nest(
            "/stats",
            Router::new()
                .route("/overview", get(stats::overview))
                .route("/models", get(stats::models))
                .route("/channels", get(stats::channels))
                .route("/daily", get(stats::daily))
                .route("/api-keys", get(stats::api_keys))
                .route("/latency", get(stats::latency))
                .route("/budgets", get(stats::list_budgets).post(stats::set_budget))
                .route("/budgets/{id}", delete(stats::delete_budget))
                .route("/logs", get(stats::logs))
                .route("/logs/models", get(stats::log_models))
                .route("/logs/{id}", get(stats::log_detail))
                .with_state(stats_state),
        )
        .nest(
            "/models/info",
            Router::new()
                .route("/", get(model_info::list).put(model_info::update))
                .route("/{model}", get(model_info::get))
                .with_state(model_info_state),
        )
        .nest(
            "/system-info",
            Router::new()
                .route("/", get(system_info::get))
                .with_state(system_info_state),
        )
        .nest(
            "/settings",
            Router::new()
                .route("/", get(settings::list))
                .route("/infra", get(settings::infra))
                .route("/{key}", put(settings::update))
                .with_state(settings_state),
        )
        .nest(
            "/backup",
            Router::new()
                .route("/export", get(backup::export))
                .route("/import", post(backup::import))
                .route("/reset", post(backup::reset))
                .with_state(backup_state),
        )
        .nest(
            "/fetch-models",
            Router::new()
                .route("/", post(fetch_models::fetch_models))
                .with_state(fetch_models_state),
        )
        .layer(middleware::from_fn(require_admin_auth))
        .layer(middleware::from_fn(crate::api::middleware::require_json));

    Router::new()
        // 请求体大小限制 50MB（多模态图片可能较大）
        .layer(DefaultBodyLimit::max(50 * 1024 * 1024))
        // 健康检查（返回初始化状态）
        .route("/api/v1/health", get(health_check))
        // 代理 API 路由
        .nest("/v1", proxy_routes(proxy_state, pool.clone()))
        // 初始化接口（无需认证）
        .nest(
            "/api/v1/init",
            Router::new()
                .route("/", post(auth::init))
                .with_state(auth_state_for_public.clone()),
        )
        // 登录接口（无需认证）
        .nest(
            "/api/v1/admin/auth",
            Router::new()
                .route("/login", post(auth::login))
                .with_state(auth_state_for_public),
        )
        // 需要认证的管理 API
        .nest("/api/v1/admin", protected_admin)
        // 静态文件服务（SPA fallback）
        .fallback(static_assets::serve)
        // CORS 中间件（从 DB 动态读取白名单）
        .layer(middleware::from_fn({
            let cors_pool = pool.clone();
            move |req, next| {
                let pool = cors_pool.clone();
                crate::api::middleware::cors::require_cors(pool, req, next)
            }
        }))
        // 注入 pool 和 JWT secret 到 extensions
        .layer(middleware::from_fn(
            move |mut req: Request<Body>, next: middleware::Next| {
                let secret = jwt_secret.clone();
                let pool = pool.clone();
                async move {
                    req.extensions_mut().insert(secret);
                    req.extensions_mut().insert(pool);
                    next.run(req).await
                }
            },
        ))
        .layer(TraceLayer::new_for_http())
}

/// 健康检查端点（返回初始化状态）
async fn health_check(axum::Extension(pool): axum::Extension<SqlitePool>) -> Json<Value> {
    let needs_setup = sqlx::query_scalar::<_, i32>("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await
        .map(|count| count == 0)
        .unwrap_or(true);

    Json(json!({
        "status": "ok",
        "version": env!("GALAXY_BUILD_VERSION"),
        "needs_setup": needs_setup
    }))
}

/// 代理 API 路由
fn proxy_routes(proxy_state: ProxyState, pool: SqlitePool) -> Router {
    use crate::api::middleware::ApiKeyCache;

    let pool_clone = pool.clone();
    let api_key_cache = ApiKeyCache::new();

    Router::new()
        .route("/chat/completions", post(chat::proxy))
        .route("/responses", post(responses::proxy))
        .route("/messages", post(messages::proxy))
        .route("/embeddings", post(embeddings::proxy))
        .route("/images/generations", post(images::proxy))
        .with_state(proxy_state)
        .route("/models", get(models::list))
        .with_state(pool)
        .layer(middleware::from_fn(
            move |mut req: Request<Body>, next: middleware::Next| {
                let pool = pool_clone.clone();
                let cache = api_key_cache.clone();
                async move {
                    req.extensions_mut().insert(pool);
                    req.extensions_mut().insert(cache);
                    next.run(req).await
                }
            },
        ))
}
