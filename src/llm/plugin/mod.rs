//! 请求插件链（relay 重构 Step C）。
//!
//! cch / tracking / cache_key 改写请求体（cch 清理缓存标记、tracking 清洗隐私、
//! cache_key 注入粘性缓存键）。thinking 响应插件推迟（单独步骤）。
//!
//! 开关：AppState 构造后一次性 `refresh` 从 settings load 到 `enabled` 缓存
//! （不每请求查 DB）；settings 更新时再次 `refresh` 重建；`master_switch` 总开关
//! （false 时所有插件跳过，紧急回滚）。

pub mod cache_key;
pub mod cch;
pub mod tracking;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::RwLock;

use crate::api::handlers::admin::channels::EndpointType;
use crate::error::proxy::ProxyError;
use crate::repository::settings_repository::SettingsRepository;

/// settings key → 插件 id 的映射（enabled 缓存按插件 id 索引，与 `RequestPlugin::id` 对齐）
const PLUGIN_SETTINGS: [(&str, &str); 4] = [
    ("plugin.cch_rewrite", "cch_rewrite"),
    ("plugin.tracking_removal", "tracking_removal"),
    ("plugin.cache_key_injection", "cache_key_injection"),
    ("plugin.thinking_fix", "thinking_fix"),
];

/// 插件执行上下文
pub struct PluginContext {
    pub upstream_endpoint: EndpointType,
    #[allow(dead_code)] // C3 校准后无插件用，保留供未来插件
    pub channel_id: String,
    /// 渠道 API Key 指纹（`ChannelInfo::key_hint`）
    pub host_key: String,
    /// User-Agent 中的客户端标识（C2 三插件未用，保留供未来插件）
    #[allow(dead_code)]
    pub client_name: Option<String>,
}

/// 请求插件执行结果
pub enum PluginResult {
    /// 继续传递（可能已改写 body）
    Continue(Value),
    /// 中止请求（暂无插件使用，保留以备未来插件 Abort）
    #[allow(dead_code)]
    Abort(ProxyError),
}

/// 请求拦截改写（cch / tracking / cache_key）
#[async_trait]
pub trait RequestPlugin: Send + Sync {
    fn id(&self) -> &'static str;
    fn matches(&self, ctx: &PluginContext) -> bool;
    /// setting 缺失时的默认开关（默认 false）
    fn default_enabled(&self) -> bool {
        false
    }
    async fn rewrite(&self, body: Value, ctx: &PluginContext) -> PluginResult;
}

/// 插件链：按顺序执行所有匹配且已启用的插件
#[derive(Clone)]
pub struct PluginChain {
    request_plugins: Arc<Vec<Box<dyn RequestPlugin>>>,
    /// 各插件开关缓存（`refresh` 时从 settings load，按插件 id 索引）
    enabled: Arc<RwLock<HashMap<&'static str, bool>>>,
    /// 全局总开关（`plugin.master_switch`，false 时所有插件跳过）
    master_switch: Arc<AtomicBool>,
}

impl PluginChain {
    /// 生产链：注册内置请求插件。router 构造后调 `refresh` 从 settings load 真实开关。
    pub fn build_default_chain() -> Self {
        Self {
            request_plugins: Arc::new(build_request_plugins()),
            enabled: Arc::new(RwLock::new(HashMap::new())),
            master_switch: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 测试用空链（无插件，master_switch=false，`apply_request` 直返原 body）
    #[cfg(test)]
    pub fn new_empty() -> Self {
        Self {
            request_plugins: Arc::new(vec![]),
            enabled: Arc::new(RwLock::new(HashMap::new())),
            master_switch: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 从 settings load 所有插件开关，重建 `enabled` + `master_switch`。
    pub async fn refresh(&self, settings: &dyn SettingsRepository) -> Result<(), sqlx::Error> {
        let mut map = HashMap::new();
        for (setting_key, plugin_id) in PLUGIN_SETTINGS {
            let on = settings
                .get(setting_key)
                .await?
                .and_then(|v| v.parse::<bool>().ok())
                .unwrap_or(false);
            map.insert(plugin_id, on);
        }
        let master = settings
            .get("plugin.master_switch")
            .await?
            .and_then(|v| v.parse::<bool>().ok())
            .unwrap_or(false);
        let mut enabled = self.enabled.write().await;
        *enabled = map;
        self.master_switch.store(master, Ordering::Relaxed);
        Ok(())
    }

    /// 应用请求插件链。`master_switch`=false 直返原 body；否则遍历匹配且启用的插件。
    pub async fn apply_request(
        &self,
        mut body: Value,
        ctx: &PluginContext,
    ) -> Result<Value, ProxyError> {
        if !self.master_switch.load(Ordering::Relaxed) {
            return Ok(body);
        }
        let enabled = self.enabled.read().await;
        for p in self.request_plugins.iter() {
            let on = *enabled.get(p.id()).unwrap_or(&p.default_enabled());
            if !on || !p.matches(ctx) {
                continue;
            }
            match p.rewrite(body, ctx).await {
                PluginResult::Continue(b) => body = b,
                PluginResult::Abort(e) => return Err(e),
            }
        }
        Ok(body)
    }
}

/// 注册内置请求插件（cch / tracking / cache_key）。
fn build_request_plugins() -> Vec<Box<dyn RequestPlugin>> {
    vec![
        Box::new(cch::CchRewriter),
        Box::new(tracking::TrackingRemover),
        Box::new(cache_key::CacheKeyInjector),
    ]
}

/// 清理 system 字段（兼容 string / `[{text}]` 两形态）。cch / tracking 共用。
pub(crate) fn clean_system(body: &mut Value, clean_fn: impl Fn(&str) -> String) {
    match body.get_mut("system") {
        Some(Value::String(s)) => *s = clean_fn(s),
        Some(Value::Array(arr)) => {
            for item in arr.iter_mut() {
                let cleaned = item.get("text").and_then(|t| t.as_str()).map(&clean_fn);
                if let Some(c) = cleaned {
                    item["text"] = Value::String(c);
                }
            }
        }
        _ => {}
    }
}
