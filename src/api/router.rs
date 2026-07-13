use std::sync::Arc;
use std::time::Instant;

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

use crate::api::handlers::admin::api_keys;
use crate::api::handlers::admin::auth;
use crate::api::handlers::admin::backup::{self, BackupState};
use crate::api::handlers::admin::channels;
use crate::api::handlers::admin::fetch_models;
use crate::api::handlers::admin::routes;
use crate::api::handlers::admin::model_info::{self, ModelsState};
use crate::api::handlers::admin::settings;
use crate::api::handlers::admin::stats;
use crate::api::handlers::admin::system_info;
use crate::api::handlers::admin::update_check;
use crate::api::handlers::proxy::{chat, embeddings, images, messages, models, responses};
use crate::api::middleware::require_admin_auth;
use crate::config::{AppConfig, QueuingConfig};
use crate::relay::cache::ProxyCache;
use crate::relay::state::ProxyState;
use crate::app_state::AppState;
use crate::static_assets;

/// 创建应用路由
pub async fn create_router(
    pool: SqlitePool,
    jwt_secret: String,
    queuing: &QueuingConfig,
    _server_addr: &str,
    config: AppConfig,
    model_registry: crate::metrics::model::ModelRegistry,
) -> Router {
    let config = Arc::new(config);
    let token_expiry_hours = config.auth.token_expiry_hours;
    let jwt_service = crate::auth::JwtService::new(&jwt_secret, token_expiry_hours);

    let shared_cache = ProxyCache::new();

    let models_state = ModelsState {
        model_registry: model_registry.clone(),
        fetch_client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client"),
    };

    let update_check_state = update_check::UpdateCheckState::from_pool(&pool).await;

    let backup_state = BackupState { pool: pool.clone() };

    let proxy_state = if queuing.enabled {
        ProxyState::new(pool.clone(), model_registry.clone())
            .await
            .with_queue(queuing.max_queue_size, queuing.queue_timeout_secs)
    } else {
        ProxyState::new(pool.clone(), model_registry.clone()).await
    };

    // v1.1.2: 统一 AppState（Settings/SystemInfo/Auth/Usage/Budget/Route/ApiKey/Channel 已切，其余 handler 后续 commit 逐个切）
    let start_time = Arc::new(Instant::now());
    let api_key_cache = crate::api::middleware::ApiKeyCache::new();
    let channel_http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("Failed to create HTTP client");
    let app_state = AppState::new(pool.clone(), config.clone(), start_time, jwt_service, shared_cache.clone(), api_key_cache, channel_http_client);

    // 需要认证的管理路由（每个 nest 独立管理状态）
    let protected_admin = Router::new()
        .nest(
            "/auth",
            Router::new()
                .route("/me", get(auth::me))
                .route("/password", put(auth::change_password))
                .with_state(app_state.clone()),
        )
        .nest(
            "/channels",
            Router::new()
                .route("/", get(channels::list).post(channels::create))
                .route("/test-endpoint", post(channels::test_endpoint))
                .route(
                    "/{id}",
                    get(channels::get)
                        .put(channels::update)
                        .delete(channels::delete),
                )
                .with_state(app_state.clone()),
        )
        .nest(
            "/routes",
            Router::new()
                .route("/", get(routes::list).post(routes::create))
                .route(
                    "/{id}",
                    get(routes::get).put(routes::update).delete(routes::delete),
                )
                .route("/{id}/items", post(routes::add_item))
                .route("/{id}/items/{item_id}", delete(routes::delete_item))
                .with_state(app_state.clone()),
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
                .with_state(app_state.clone()),
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
                .route("/logs", get(stats::logs))
                .route("/logs/models", get(stats::log_models))
                .route("/logs/{id}", get(stats::log_detail))
                .with_state(app_state.clone()),
        )
        .nest(
            "/budgets",
            Router::new()
                .route("/", get(stats::list_budgets).post(stats::set_budget))
                .route("/{id}", delete(stats::delete_budget))
                .with_state(app_state.clone()),
        )
        .nest(
            "/models",
            Router::new()
                .route("/", get(model_info::list).put(model_info::update))
                .route("/fetch", post(fetch_models::fetch_models))
                .route("/{model}", get(model_info::get))
                .with_state(models_state),
        )
        .nest(
            "/system-info",
            Router::new()
                .route("/", get(system_info::get))
                .with_state(app_state.clone()),
        )
        .nest(
            "/update-check",
            Router::new()
                .route("/", get(update_check::get))
                .with_state(update_check_state),
        )
        .nest(
            "/settings",
            Router::new()
                .route("/", get(settings::list))
                .route("/infra", get(settings::infra))
                .route("/{key}", put(settings::update))
                .with_state(app_state.clone()),
        )
        .nest(
            "/backups",
            Router::new()
                .route("/", get(backup::export).post(backup::import).delete(backup::reset))
                .with_state(backup_state),
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
                .with_state(app_state.clone()),
        )
        // 登录接口（无需认证）
        .nest(
            "/api/v1/admin/auth",
            Router::new()
                .route("/login", post(auth::login))
                .with_state(app_state.clone()),
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
        // 注入 pool / JWT secret / start_time 到 extensions
        .layer({
            let start_time = Arc::new(Instant::now());
            middleware::from_fn(
                move |mut req: Request<Body>, next: middleware::Next| {
                    let secret = jwt_secret.clone();
                    let pool = pool.clone();
                    let start = start_time.clone();
                    async move {
                        req.extensions_mut().insert(secret);
                        req.extensions_mut().insert(pool);
                        req.extensions_mut().insert(start);
                        next.run(req).await
                    }
                },
            )
        })
        .layer(TraceLayer::new_for_http())
}

/// 健康检查端点（返回初始化状态 + 运行时长）
async fn health_check(
    axum::Extension(pool): axum::Extension<SqlitePool>,
    axum::Extension(start_time): axum::Extension<Arc<Instant>>,
) -> Json<Value> {
    let needs_setup = sqlx::query_scalar::<_, i32>("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await
        .map(|count| count == 0)
        .unwrap_or(true);

    Json(json!({
        "status": "ok",
        "version": env!("GALAXY_BUILD_VERSION"),
        "needs_setup": needs_setup,
        "uptime_seconds": start_time.elapsed().as_secs()
    }))
}

/// 代理 API 路由
fn proxy_routes(proxy_state: ProxyState, pool: SqlitePool) -> Router {
    use crate::api::middleware::ApiKeyCache;

    let pool_clone = pool.clone();
    let api_key_cache = ApiKeyCache::new();
    let stats_recorder = proxy_state.stats_recorder.clone();

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
                let recorder = stats_recorder.clone();
                async move {
                    req.extensions_mut().insert(pool);
                    req.extensions_mut().insert(cache);
                    // 供 ApiKeyAuth 鉴权失败时写 usage_logs（审计兜底）
                    req.extensions_mut().insert(recorder);
                    next.run(req).await
                }
            },
        ))
}
