use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt};

mod api;
mod auth;
mod config;
mod db;
mod metrics;
mod protocol;
mod relay;
mod scheduler;
mod static_assets;

use config::AppConfig;
use db::Database;

/// Galaxy Router - AI 协议互转代理网关
#[derive(Parser, Debug)]
#[command(name = "galaxy-router", version = env!("GALAXY_BUILD_VERSION"), about)]
struct Cli {
    /// 配置文件路径
    #[arg(short, long, default_value = "config.toml")]
    config: PathBuf,

    /// 监听端口（覆盖配置文件）
    #[arg(short, long)]
    port: Option<u16>,

    /// 监听地址（覆盖配置文件）
    #[arg(long)]
    host: Option<String>,

    /// 日志级别
    #[arg(short, long)]
    log_level: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let config = AppConfig::load(&cli.config)?;

    let log_level = cli.log_level.as_deref().unwrap_or(&config.logging.level);
    let _guard = init_logging(log_level, &config.logging)?;

    info!("Configuration loaded from {:?}", cli.config);

    let config = apply_cli_overrides(config, &cli);

    let database = Database::new(&config.database_url()).await?;
    info!("Database initialized");

    let config = ensure_jwt_secret(config, &cli.config)?;

    // 初始化模型注册表：从 DB 加载，空则回退缓存文件
    let cache_path = PathBuf::from(&config.pricing.cache_path);
    let providers = config.pricing.providers.clone();
    let model_registry = metrics::model::ModelRegistry::new(database.pool().clone());

    model_registry
        .load_from_db()
        .await
        .map_err(|e| anyhow::anyhow!("加载 DB 模型信息失败: {}", e))?;

    {
        let model_count = model_registry.get_all_models().await.len();
        if model_count == 0 {
            info!("DB 无模型信息，尝试从缓存文件加载");
            if let Err(e) = model_registry.load_from_cache(&cache_path).await {
                tracing::warn!("加载缓存失败: {}", e);
            }
        }
    }

    // 后台尝试远程刷新
    let bg_registry = model_registry.clone();
    let bg_providers = providers.clone();
    tokio::spawn(async move {
        if let Err(e) = bg_registry
            .fetch_remote_pricing(&cache_path, &bg_providers)
            .await
        {
            tracing::warn!("远程模型信息刷新失败: {}", e);
        }
    });

    info!("Model registry initialized");

    // 启动后台调度器
    let lb_state = scheduler::state::LoadBalancerState::new();
    let rate_limiter = relay::ratelimit::RateLimiter::new();
    let scheduler = Arc::new(relay::scheduler_task::Scheduler::new(
        lb_state,
        rate_limiter.clone(),
        database.pool().clone(),
    ));
    scheduler.start();
    info!("Scheduler started");

    // 模型信息定时刷新
    let pricing_refresher = Arc::new(metrics::pricing::PricingRefresher::new(
        model_registry.clone(),
        PathBuf::from(&config.pricing.cache_path),
        config.pricing.providers.clone(),
        config.pricing.refresh_interval_hours,
    ));
    pricing_refresher.start();
    info!("Pricing refresher started");

    // 创建路由
    let addr = config.server_addr();
    let jwt_secret = config.auth.jwt_secret.clone();
    let queuing = config.queuing.clone();
    let app = api::create_router(
        database.pool().clone(),
        jwt_secret,
        &queuing,
        &addr,
        config,
        model_registry,
    )
    .await;

    info!("Starting server on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

fn init_logging(
    log_level: &str,
    logging_config: &config::LoggingConfig,
) -> Result<Option<tracing_appender::non_blocking::WorkerGuard>> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));

    let mut guard = None;

    if logging_config.file {
        let path = PathBuf::from(&logging_config.file_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let prefix = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("galaxy-router");
        let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("log");
        let directory = path.parent().unwrap_or_else(|| std::path::Path::new("."));

        let file_appender = match logging_config.rotation.as_str() {
            "hourly" => tracing_appender::rolling::hourly(directory, prefix),
            "never" => {
                tracing_appender::rolling::never(directory, path.file_name().unwrap_or_default())
            }
            _ => tracing_appender::rolling::daily(directory, prefix),
        };
        // 如果配置了扩展名且不是 "never" 模式，tracing_appender 会自动追加日期后缀
        let _ = extension; // 保留变量避免 unused warning
        let (non_blocking, nb_guard) = tracing_appender::non_blocking(file_appender);
        guard = Some(nb_guard);

        match logging_config.format.as_str() {
            "json" => {
                fmt()
                    .with_env_filter(filter)
                    .with_target(true)
                    .with_thread_ids(true)
                    .with_file(true)
                    .with_line_number(true)
                    .json()
                    .with_writer(non_blocking)
                    .init();
            }
            _ => {
                fmt()
                    .with_env_filter(filter)
                    .with_target(true)
                    .with_thread_ids(true)
                    .with_file(true)
                    .with_line_number(true)
                    .with_writer(non_blocking)
                    .init();
            }
        }
    } else {
        match logging_config.format.as_str() {
            "json" => {
                fmt()
                    .with_env_filter(filter)
                    .with_target(true)
                    .with_thread_ids(true)
                    .with_file(true)
                    .with_line_number(true)
                    .json()
                    .init();
            }
            _ => {
                fmt()
                    .with_env_filter(filter)
                    .with_target(true)
                    .with_thread_ids(true)
                    .with_file(true)
                    .with_line_number(true)
                    .init();
            }
        }
    }

    Ok(guard)
}

fn apply_cli_overrides(mut config: AppConfig, cli: &Cli) -> AppConfig {
    if let Some(port) = cli.port {
        config.server.port = port;
    }
    if let Some(host) = &cli.host {
        config.server.host = host.clone();
    }
    config
}

fn ensure_jwt_secret(mut config: AppConfig, config_path: &PathBuf) -> Result<AppConfig> {
    if config.auth.jwt_secret.is_empty() {
        use rand::Rng;
        let secret: String = rand::rng()
            .sample_iter(&rand::distr::Alphanumeric)
            .take(32)
            .map(char::from)
            .collect();

        info!("Generated new JWT secret");

        let content = std::fs::read_to_string(config_path)?;
        let mut toml_value: toml::Value = toml::from_str(&content)?;

        if let Some(auth) = toml_value.get_mut("auth")
            && let Some(table) = auth.as_table_mut()
        {
            table.insert(
                "jwt_secret".to_string(),
                toml::Value::String(secret.clone()),
            );
        }

        std::fs::write(config_path, toml::to_string_pretty(&toml_value)?)?;
        config.auth.jwt_secret = secret;
    }

    Ok(config)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to listen for ctrl+c");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to listen for SIGTERM")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    info!("收到关停信号，正在优雅关闭...");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_config() -> AppConfig {
        AppConfig {
            server: config::ServerConfig {
                host: "127.0.0.1".into(),
                port: 8080,
                timezone_offset: 0,
            },
            database: config::DatabaseConfig {
                path: "/tmp/galaxy_test_main.db".into(),
            },
            logging: config::LoggingConfig {
                level: "info".into(),
                format: "compact".into(),
                file: false,
                file_path: "/tmp/galaxy_test_main.log".into(),
                rotation: "daily".into(),
                max_files: 30,
            },
            auth: config::AuthConfig {
                jwt_secret: String::new(),
                token_expiry_hours: 24,
            },
            queuing: config::QueuingConfig::default(),
            pricing: config::PricingTomlConfig {
                cache_path: "/tmp/galaxy_test_main_pricing.json".into(),
                refresh_interval_hours: 24,
                providers: vec![],
            },
        }
    }

    #[test]
    fn apply_cli_overrides_no_args_leaves_config_unchanged() {
        let config = make_config();
        let cli = Cli {
            config: PathBuf::from("config.toml"),
            port: None,
            host: None,
            log_level: None,
        };
        let out = apply_cli_overrides(config.clone(), &cli);
        assert_eq!(out.server.port, 8080);
        assert_eq!(out.server.host, "127.0.0.1");
    }

    #[test]
    fn apply_cli_overrides_port_and_host() {
        let config = make_config();
        let cli = Cli {
            config: PathBuf::from("config.toml"),
            port: Some(9999),
            host: Some("0.0.0.0".into()),
            log_level: None,
        };
        let out = apply_cli_overrides(config, &cli);
        assert_eq!(out.server.port, 9999);
        assert_eq!(out.server.host, "0.0.0.0");
    }

    #[test]
    fn ensure_jwt_secret_preserves_existing() {
        let mut config = make_config();
        config.auth.jwt_secret = "preset-secret".into();
        let path = PathBuf::from("/tmp/galaxy_test_main_nofile.toml");
        let _ = std::fs::remove_file(&path);

        let out = ensure_jwt_secret(config, &path).unwrap();
        assert_eq!(out.auth.jwt_secret, "preset-secret");
        // 不应写文件
        assert!(!path.exists());
    }

    #[test]
    fn ensure_jwt_secret_generates_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(
            f,
            r#"
[server]
host = "0.0.0.0"
port = 8080

[auth]
jwt_secret = ""
token_expiry_hours = 24
"#
        )
        .unwrap();
        drop(f);

        let mut config = make_config();
        config.auth.jwt_secret = String::new();
        let out = ensure_jwt_secret(config, &path).unwrap();
        assert_eq!(out.auth.jwt_secret.len(), 32);

        // 重新读回文件，应包含新 secret
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("jwt_secret ="));
        assert!(!content.contains(r#"jwt_secret = """#));
    }

    #[test]
    fn cli_parse_defaults() {
        let cli = Cli::try_parse_from(["galaxy-router"]).unwrap();
        assert_eq!(cli.config, PathBuf::from("config.toml"));
        assert_eq!(cli.port, None);
        assert_eq!(cli.host, None);
        assert_eq!(cli.log_level, None);
    }

    #[test]
    fn cli_parse_with_overrides() {
        let cli = Cli::try_parse_from([
            "galaxy-router",
            "--config",
            "/etc/galaxy.toml",
            "--port",
            "7777",
            "--host",
            "0.0.0.0",
            "--log-level",
            "debug",
        ])
        .unwrap();
        assert_eq!(cli.config, PathBuf::from("/etc/galaxy.toml"));
        assert_eq!(cli.port, Some(7777));
        assert_eq!(cli.host.as_deref(), Some("0.0.0.0"));
        assert_eq!(cli.log_level.as_deref(), Some("debug"));
    }
}
