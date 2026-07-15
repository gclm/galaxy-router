use anyhow::Result;
use serde::Deserialize;
use std::path::Path;

/// 应用配置（从 TOML 文件加载）
#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub logging: LoggingConfig,
    pub auth: AuthConfig,
    #[serde(default)]
    pub queuing: QueuingConfig,
    #[serde(default)]
    pub pricing: PricingTomlConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    #[serde(default = "default_timezone_offset")]
    pub timezone_offset: i32,
}

fn default_timezone_offset() -> i32 {
    8
}

#[derive(Debug, Deserialize, Clone)]
pub struct DatabaseConfig {
    pub path: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LoggingConfig {
    pub level: String,
    pub format: String,
    pub file: bool,
    pub file_path: String,
    /// 日志轮转策略: "daily", "hourly", "never"（默认 daily）
    #[serde(default = "default_rotation")]
    pub rotation: String,
    /// 最大保留文件数（默认 30）
    #[serde(default = "default_max_files")]
    #[allow(dead_code)]
    pub max_files: usize,
}

fn default_rotation() -> String {
    "daily".to_string()
}

fn default_max_files() -> usize {
    30
}

#[derive(Debug, Deserialize, Clone)]
pub struct AuthConfig {
    pub jwt_secret: String,
    pub token_expiry_hours: u64,
}

/// 排队配置
#[derive(Debug, Deserialize, Clone)]
pub struct QueuingConfig {
    /// 是否启用排队（默认关闭，直接返回 429）
    #[serde(default)]
    pub enabled: bool,
    /// 最大排队请求数
    #[serde(default = "default_max_queue_size")]
    pub max_queue_size: usize,
    /// 排队超时时间（秒）
    #[serde(default = "default_queue_timeout_secs")]
    pub queue_timeout_secs: u64,
}

impl Default for QueuingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_queue_size: default_max_queue_size(),
            queue_timeout_secs: default_queue_timeout_secs(),
        }
    }
}

fn default_max_queue_size() -> usize {
    100
}

fn default_queue_timeout_secs() -> u64 {
    30
}

/// 定价配置（config.toml）
#[derive(Debug, Deserialize, Clone)]
pub struct PricingTomlConfig {
    /// 本地缓存文件路径（相对于工作目录）
    #[serde(default = "default_pricing_cache_path")]
    pub cache_path: String,
    /// 远程刷新间隔（小时）
    #[serde(default = "default_pricing_refresh_hours")]
    pub refresh_interval_hours: u64,
    /// 启用的 provider 白名单
    #[serde(default = "default_pricing_providers")]
    pub providers: Vec<String>,
}

impl Default for PricingTomlConfig {
    fn default() -> Self {
        Self {
            cache_path: default_pricing_cache_path(),
            refresh_interval_hours: default_pricing_refresh_hours(),
            providers: default_pricing_providers(),
        }
    }
}

fn default_pricing_providers() -> Vec<String> {
    vec![
        "openai".into(),
        "anthropic".into(),
        "deepseek".into(),
        "google".into(),
        "zai".into(),
        "minimax".into(),
        "xai".into(),
        "alibaba".into(),
        "moonshotai".into(),
        "xiaomi".into(),
        "stepfun".into(),
    ]
}

fn default_pricing_cache_path() -> String {
    "data/pricing_cache.json".to_string()
}

fn default_pricing_refresh_hours() -> u64 {
    24
}

impl AppConfig {
    /// 从配置文件加载配置
    pub fn load(path: &Path) -> Result<Self> {
        let config = config::Config::builder()
            .add_source(config::File::from(path))
            .add_source(config::Environment::with_prefix("GALAXY_PROXY"))
            .build()?;

        let app_config: AppConfig = config.try_deserialize()?;
        Ok(app_config)
    }

    /// 获取服务器地址
    pub fn server_addr(&self) -> String {
        format!("{}:{}", self.server.host, self.server.port)
    }

    /// 获取数据库 URL
    pub fn database_url(&self) -> String {
        format!("sqlite:{}?mode=rwc", self.database.path)
    }
}
