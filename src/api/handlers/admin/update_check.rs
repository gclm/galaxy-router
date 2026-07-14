//! 版本更新检查：对比当前版本与 GitHub latest release，判断是否有新版本。
//!
//! 仅取"检查"部分（不自动更新）。代理复用 settings 的 `proxy.url`（与上游渠道一致），
//! GitHub 仓库从 settings 的 `github.repo` 读取，便于 fork 自定义。结果在服务端短期缓存，
//! 请求超时及时降级。代理构建参考 `relay/state.rs`。

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::app_state::AppState;
use crate::error::app::{ApiError, ApiResponse};
use crate::repository::settings_repository::{SettingsRepository, SqliteSettingsRepository};

/// GitHub API 域名（固定；owner/repo 在 settings.github.repo 配置）
const GITHUB_API_BASE: &str = "https://api.github.com";
const DEFAULT_GITHUB_REPO: &str = "gclm/galaxy-router";
/// 检查结果缓存时长（秒）。缓存命中秒回、不打 GitHub、不被限流。
pub const UPDATE_CHECK_TTL_SECS: u64 = 600;
/// 调用 GitHub 的 HTTP 超时（秒）。国内 api.github.com 不稳，超时及时降级。
pub const HTTP_TIMEOUT_SECS: u64 = 10;

#[derive(Clone)]
pub struct UpdateCheckContext {
    pub http_client: reqwest::Client,
    github_repo: String,
    mirror: Option<String>,
    api_base: String,
    cache: Arc<std::sync::RwLock<Option<CachedResult>>>,
}

impl UpdateCheckContext {
    /// 生产：从 settings 读取代理 + GitHub 仓库 + 镜像，构建客户端（在 router 启动时调用）。
    pub async fn from_pool(pool: &SqlitePool) -> Self {
        let settings = SqliteSettingsRepository::new(pool.clone());
        let proxy_url = read_proxy_url(&settings).await;
        let github_repo = settings
            .get("github.repo")
            .await
            .ok()
            .flatten()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_GITHUB_REPO.to_string());
        let mirror = settings
            .get("update.mirror")
            .await
            .ok()
            .flatten()
            .filter(|s| !s.is_empty());
        Self {
            http_client: build_proxied_client(proxy_url),
            github_repo,
            mirror,
            api_base: GITHUB_API_BASE.to_string(),
            cache: Arc::new(std::sync::RwLock::new(None)),
        }
    }

    /// 测试：注入 client + api_base + github_repo + mirror
    #[cfg(test)]
    fn with_client(
        client: reqwest::Client,
        api_base: &str,
        github_repo: &str,
        mirror: Option<&str>,
    ) -> Self {
        Self {
            http_client: client,
            github_repo: github_repo.to_string(),
            mirror: mirror.map(|s| s.to_string()),
            api_base: api_base.to_string(),
            cache: Arc::new(std::sync::RwLock::new(None)),
        }
    }

    /// 执行检查（缓存 + GitHub fetch + 镜像 fallback）。handler 与测试共用。
    pub async fn check(&self) -> Result<UpdateCheckResponse, (StatusCode, Json<ApiError>)> {
        // 1. 缓存未过期 → 直接返回
        {
            let cache = self.cache.read().unwrap();
            if let Some(cached) = cache.as_ref()
                && cached.fetched_at.elapsed().as_secs() < UPDATE_CHECK_TTL_SECS
            {
                return Ok(cached.response.clone());
            }
        }

        // 2. 缓存过期，调 GitHub（三级 fallback：api → 镜像 release-info.json）
        let api_url = format!(
            "{}/repos/{}/releases/latest",
            self.api_base, self.github_repo
        );
        let response = match fetch_and_compare(&self.http_client, &api_url).await {
            Ok(r) => r,
            Err(api_err) => match self.mirror.as_deref().filter(|m| !m.is_empty()) {
                Some(mirror) => {
                    // 镜像下载 release-info.json（ghfast/gh-proxy 加速 github.com 下载）
                    let mirror_url = apply_mirror(
                        mirror,
                        &format!(
                            "https://github.com/{}/releases/latest/download/release-info.json",
                            self.github_repo
                        ),
                    );
                    tracing::info!("api.github.com 失败，尝试镜像: {}", mirror_url);
                    fetch_and_compare(&self.http_client, &mirror_url).await?
                }
                None => return Err(api_err),
            },
        };

        // 3. 写缓存
        {
            let mut cache = self.cache.write().unwrap();
            *cache = Some(CachedResult {
                fetched_at: Instant::now(),
                response: response.clone(),
            });
        }

        Ok(response)
    }
}

struct CachedResult {
    fetched_at: Instant,
    response: UpdateCheckResponse,
}

#[derive(Debug, Serialize, Clone)]
pub struct UpdateCheckResponse {
    pub current_version: String,
    pub latest_version: String,
    pub has_update: bool,
    pub release_url: String,
    pub release_notes: String,
    pub published_at: String,
    pub checked_at: i64,
}

/// GitHub `/releases/latest` 返回字段（只取需要的）
#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    body: Option<String>,
    published_at: String,
}

/// 检查更新
pub async fn get(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<UpdateCheckResponse>>, (StatusCode, Json<ApiError>)> {
    let response = state.update_check.check().await?;
    Ok(Json(ApiResponse::success(response)))
}

/// 调用 GitHub latest release + 解析 + 版本比较（核心逻辑，测试注入 client + url）。
async fn fetch_and_compare(
    client: &reqwest::Client,
    url: &str,
) -> Result<UpdateCheckResponse, (StatusCode, Json<ApiError>)> {
    let resp = client
        .get(url)
        .header("User-Agent", "galaxy-router") // GitHub API 强制要求 UA
        .send()
        .await
        .map_err(|e| ApiError::internal_error(format!("检查更新失败：无法连接 GitHub（{e}）")))?;

    if !resp.status().is_success() {
        return Err(ApiError::internal_error(format!(
            "检查更新失败：GitHub 返回 {}",
            resp.status()
        )));
    }

    let release: GithubRelease = resp
        .json()
        .await
        .map_err(|e| ApiError::internal_error(format!("检查更新失败：解析响应失败（{e}）")))?;

    let current_raw = env!("GALAXY_BUILD_VERSION");
    Ok(UpdateCheckResponse {
        current_version: display_version(current_raw),
        latest_version: display_version(&release.tag_name),
        has_update: has_update(current_raw, &release.tag_name),
        release_url: release.html_url,
        release_notes: release.body.unwrap_or_default(),
        published_at: release.published_at,
        checked_at: chrono::Utc::now().timestamp(),
    })
}

/// 应用下载镜像前缀：`{prefix}/{original_url}`（prefix 去掉末尾 /）。
/// 如 `https://ghfast.top/` + `https://github.com/...` → `https://ghfast.top/https://github.com/...`
fn apply_mirror(prefix: &str, url: &str) -> String {
    let prefix = prefix.trim_end_matches('/');
    format!("{prefix}/{url}")
}

/// 读取代理 URL：仅当 proxy.enabled=true 且 proxy.url 非空时返回
async fn read_proxy_url(settings: &SqliteSettingsRepository) -> Option<String> {
    let enabled = settings
        .get("proxy.enabled")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse::<bool>().ok())
        .unwrap_or(false);
    if !enabled {
        return None;
    }
    settings
        .get("proxy.url")
        .await
        .ok()
        .flatten()
        .filter(|v| !v.is_empty())
}

/// 构建带可选代理的 HTTP 客户端（未配代理时 no_proxy，与上游渠道行为一致）
fn build_proxied_client(proxy_url: Option<String>) -> reqwest::Client {
    let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(HTTP_TIMEOUT_SECS));
    match proxy_url {
        Some(url) => match reqwest::Proxy::all(&url) {
            Ok(proxy) => {
                tracing::info!("版本检查启用代理: {}", url);
                builder = builder.proxy(proxy);
            }
            Err(e) => {
                tracing::warn!("版本检查代理配置无效，忽略代理: {}", e);
                builder = builder.no_proxy();
            }
        },
        None => {
            builder = builder.no_proxy();
        }
    }
    builder
        .build()
        .expect("Failed to create update check HTTP client")
}

/// 解析版本号为数值段数组。
/// - 去 `v` 前缀（`v1.0.3` → `1.0.3`）
/// - 去 build metadata（`+` 后部分，`1.0.2+homebrew.x` → `1.0.2`）
/// - 按点分段解析为 u64；任一段非数字返回 None
fn parse_version(s: &str) -> Option<Vec<u64>> {
    let s = s.strip_prefix('v').unwrap_or(s);
    let s = s.split('+').next()?;
    let mut parts = Vec::new();
    for p in s.split('.') {
        parts.push(p.parse::<u64>().ok()?);
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts)
}

/// latest 是否严格大于 current（有新版本）。
/// 任一解析失败保守返回 false（避免误报促使用户"更新"到无法判断的版本）。
fn has_update(current: &str, latest: &str) -> bool {
    match (parse_version(current), parse_version(latest)) {
        (Some(c), Some(l)) => l > c, // Vec<u64> 逐段数值比较
        _ => false,
    }
}

/// 展示用版本号：去 `v` 前缀和 build metadata，保留 `x.y.z`。
fn display_version(s: &str) -> String {
    let s = s.strip_prefix('v').unwrap_or(s);
    s.split('+').next().unwrap_or(s).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_strips_v_prefix_and_build_metadata() {
        assert_eq!(parse_version("v1.0.3"), Some(vec![1, 0, 3]));
        assert_eq!(
            parse_version("1.0.2+homebrew.20260611"),
            Some(vec![1, 0, 2])
        );
        assert_eq!(parse_version("1.10.0"), Some(vec![1, 10, 0]));
    }

    #[test]
    fn parse_returns_none_for_invalid() {
        assert_eq!(parse_version("1.0.x"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn has_update_true_when_latest_newer() {
        assert!(has_update("1.0.2+homebrew.x", "v1.0.3"));
        assert!(has_update("1.0.2", "1.0.3"));
        assert!(has_update("1.0.2", "v1.0.3")); // v 前缀
    }

    #[test]
    fn has_update_false_when_equal() {
        assert!(!has_update("1.0.2", "1.0.2"));
        assert!(!has_update("1.0.2+homebrew.x", "v1.0.2")); // build metadata 不算新
    }

    #[test]
    fn has_update_handles_multidigit_segments() {
        // 逐段数值比较，非字符串：1.10.0 > 1.9.0
        assert!(has_update("1.9.0", "1.10.0"));
        assert!(!has_update("1.10.0", "1.9.0"));
    }

    #[test]
    fn has_update_false_when_parsing_fails() {
        assert!(!has_update("1.0.2", "v1.0.x"));
        assert!(!has_update("invalid", "1.0.3"));
    }

    #[test]
    fn display_strips_prefix_and_metadata() {
        assert_eq!(display_version("v1.0.3"), "1.0.3");
        assert_eq!(display_version("1.0.2+homebrew.20260611"), "1.0.2");
    }

    // ---------- handler 级集成测试（wiremock 模拟 GitHub）----------
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn gh_release_json(tag: &str) -> String {
        serde_json::json!({
            "tag_name": tag,
            "html_url": format!("https://github.com/gclm/galaxy-router/releases/tag/{tag}"),
            "body": "release notes",
            "published_at": "2026-06-01T00:00:00Z",
        })
        .to_string()
    }

    async fn call_get_ok(state: UpdateCheckContext) -> serde_json::Value {
        let resp = state.check().await.expect("check 应成功");
        serde_json::to_value(ApiResponse::success(resp)).unwrap()
    }

    #[tokio::test]
    async fn handler_detects_new_version() {
        // latest v99.0.0 必然 > 编译期当前版本 → has_update=true
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/test/repo/releases/latest"))
            .respond_with(ResponseTemplate::new(200).set_body_string(gh_release_json("v99.0.0")))
            .mount(&server)
            .await;
        let state = UpdateCheckContext::with_client(
            reqwest::Client::new(),
            &server.uri(),
            "test/repo",
            None,
        );
        let resp = call_get_ok(state).await;
        assert_eq!(resp["code"], 0);
        assert_eq!(resp["data"]["has_update"], true);
        assert_eq!(resp["data"]["latest_version"], "99.0.0");
        assert_eq!(resp["data"]["release_notes"], "release notes");
    }

    #[tokio::test]
    async fn handler_no_update_when_latest_older() {
        // latest v0.0.1 < 当前 → has_update=false
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/test/repo/releases/latest"))
            .respond_with(ResponseTemplate::new(200).set_body_string(gh_release_json("v0.0.1")))
            .mount(&server)
            .await;
        let state = UpdateCheckContext::with_client(
            reqwest::Client::new(),
            &server.uri(),
            "test/repo",
            None,
        );
        let resp = call_get_ok(state).await;
        assert_eq!(resp["data"]["has_update"], false);
    }

    #[tokio::test]
    async fn cache_hit_skips_refetch() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/test/repo/releases/latest"))
            .respond_with(ResponseTemplate::new(200).set_body_string(gh_release_json("v99.0.0")))
            .expect(1) // 缓存命中 → 第二次不再请求
            .mount(&server)
            .await;
        let state = UpdateCheckContext::with_client(
            reqwest::Client::new(),
            &server.uri(),
            "test/repo",
            None,
        );
        let r1 = call_get_ok(state.clone()).await;
        let r2 = call_get_ok(state).await;
        assert_eq!(r1["data"]["latest_version"], r2["data"]["latest_version"]);
    }

    #[tokio::test]
    async fn returns_error_when_github_fails() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/test/repo/releases/latest"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let state = UpdateCheckContext::with_client(
            reqwest::Client::new(),
            &server.uri(),
            "test/repo",
            None,
        );
        let result = state.check().await;
        assert!(result.is_err(), "GitHub 5xx 应返回错误而非 panic");
    }

    #[tokio::test]
    async fn mirror_fallback_when_api_fails() {
        // api 失败 → 镜像下载 release-info.json 成功
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/test/repo/releases/latest"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(
                "/https://github.com/test/repo/releases/latest/download/release-info.json",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(gh_release_json("v99.0.0")))
            .mount(&server)
            .await;
        let state = UpdateCheckContext::with_client(
            reqwest::Client::new(),
            &server.uri(),
            "test/repo",
            Some(&server.uri()),
        );
        let resp = call_get_ok(state).await;
        assert_eq!(resp["data"]["has_update"], true);
        assert_eq!(resp["data"]["latest_version"], "99.0.0");
    }
}
