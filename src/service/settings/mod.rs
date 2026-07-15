//! 设置 service：白名单校验 + 设置更新 + 插件链刷新副作用（D6 归位）。
//!
//! handler 只做 HTTP 适配；ALLOWED_SETTING_KEYS 白名单、plugin.* 的 PluginChain.refresh 在此。

use std::sync::Arc;

use crate::domain::setting::SettingResponse;
use crate::llm::plugin::PluginChain;
use crate::repository::settings_repository::SettingsRepository;

/// 允许通过 API 更新的设置项白名单
const ALLOWED_SETTING_KEYS: &[&str] = &[
    "scheduler.top_k",
    "scheduler.score_weights",
    "sticky_session.enabled",
    "sticky_session.ttl_seconds",
    "proxy.enabled",
    "proxy.url",
    "cors.allow_origins",
    "github.repo",
    "update.mirror",
    "plugin.cch_rewrite",
    "plugin.tracking_removal",
    "plugin.cache_key_injection",
    "plugin.thinking_fix",
    "plugin.master_switch",
    "usage.record_content",
];

#[derive(Clone)]
pub struct SettingsService {
    repo: Arc<dyn SettingsRepository>,
    plugin_chain: PluginChain,
}

impl SettingsService {
    pub fn new(repo: Arc<dyn SettingsRepository>, plugin_chain: PluginChain) -> Self {
        Self { repo, plugin_chain }
    }

    pub async fn list(&self) -> Result<Vec<SettingResponse>, sqlx::Error> {
        self.repo.list().await
    }

    pub async fn update(&self, key: &str, value: &str) -> Result<(), SettingsError> {
        if !ALLOWED_SETTING_KEYS.contains(&key) {
            return Err(SettingsError::KeyNotAllowed(key.to_string()));
        }
        let updated = self
            .repo
            .update(key, value)
            .await
            .map_err(|e| SettingsError::Internal(e.to_string()))?;
        if !updated {
            return Err(SettingsError::NotFound(key.to_string()));
        }
        // 插件开关变更：重建 PluginChain 内存缓存
        if key.starts_with("plugin.") {
            self.plugin_chain
                .refresh(&*self.repo)
                .await
                .map_err(|e| SettingsError::Internal(e.to_string()))?;
        }
        Ok(())
    }
}

pub enum SettingsError {
    KeyNotAllowed(String),
    NotFound(String),
    Internal(String),
}
