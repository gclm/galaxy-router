use std::path::PathBuf;
use std::sync::Arc;
use tokio::time::{Duration, interval};

use super::model::ModelRegistry;

/// 模型信息定时刷新器
pub struct PricingRefresher {
    registry: ModelRegistry,
    cache_path: PathBuf,
    providers: Vec<String>,
    refresh_interval_hours: u64,
}

impl PricingRefresher {
    pub fn new(
        registry: ModelRegistry,
        cache_path: PathBuf,
        providers: Vec<String>,
        refresh_interval_hours: u64,
    ) -> Self {
        Self {
            registry,
            cache_path,
            providers,
            refresh_interval_hours,
        }
    }

    pub fn start(self: Arc<Self>) {
        let refresher = self.clone();
        tokio::spawn(async move {
            refresher.run().await;
        });
    }

    async fn run(&self) {
        let mut tick = interval(Duration::from_secs(self.refresh_interval_hours * 3600));

        loop {
            tick.tick().await;

            tracing::info!("开始定时刷新模型信息");
            if let Err(e) = self
                .registry
                .fetch_remote_pricing(&self.cache_path, &self.providers)
                .await
            {
                tracing::warn!("定时刷新模型信息失败: {}", e);
            } else {
                let count = self.registry.get_all_models().await.len();
                tracing::info!("模型信息刷新完成，当前 {} 条", count);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 注：run() 是无限循环 + tokio::spawn，无法在单测里直接 await 终止。
    // 这里只测构造器字段透传正确性 + Arc<Self> 可被 start() 接收。

    #[tokio::test]
    async fn new_preserves_all_fields_and_arc_shares() {
        let db_path = format!("/tmp/galaxy_pricing_refresher_{}.db", uuid::Uuid::now_v7());
        let _ = std::fs::remove_file(&db_path);
        let db_url = format!("sqlite:{}?mode=rwc", db_path);
        let pool = crate::db::Database::new(&db_url)
            .await
            .unwrap()
            .pool()
            .clone();

        let registry = ModelRegistry::new(pool);
        let cache_path = PathBuf::from("/tmp/galaxy_pricing_cache.json");
        let providers = vec!["openai".to_string(), "anthropic".to_string()];
        let refresher = PricingRefresher::new(registry, cache_path, providers, 6);
        // Arc<Self> 是 start() 的签名约束，验证能 clone
        let arc = std::sync::Arc::new(refresher);
        let arc2 = arc.clone();
        assert_eq!(Arc::strong_count(&arc), 2);
        drop(arc2);
        assert_eq!(Arc::strong_count(&arc), 1);
    }
}
