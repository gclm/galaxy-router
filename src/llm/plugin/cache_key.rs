//! OpenAI Responses `prompt_cache_key` 粘性路由注入。
//!
//! 借鉴 GT AI Gateway `responsesPromptCacheKeyRewriter`。注入稳定 key，使无状态客户端
//! 也能命中粘性缓存（同 channel + key → 同缓存节点）。

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{PluginContext, PluginResult, RequestPlugin};
use crate::api::handlers::admin::channels::EndpointType;

pub struct CacheKeyInjector;

#[async_trait]
impl RequestPlugin for CacheKeyInjector {
    fn id(&self) -> &'static str {
        "cache_key_injection"
    }
    fn matches(&self, ctx: &PluginContext) -> bool {
        ctx.upstream_endpoint == EndpointType::OpenAiResponse
    }
    async fn rewrite(&self, mut body: Value, ctx: &PluginContext) -> PluginResult {
        body["prompt_cache_key"] = json!(format!("galaxy:{}:{}", ctx.channel_id, ctx.host_key));
        PluginResult::Continue(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(endpoint: EndpointType) -> PluginContext {
        PluginContext {
            upstream_endpoint: endpoint,
            channel_id: "ch1".into(),
            host_key: "key-hint".into(),
            client_name: None,
        }
    }

    #[tokio::test]
    async fn injects_key_for_responses() {
        let body = json!({"model": "gpt-4o", "input": "hi"});
        let out = CacheKeyInjector.rewrite(body, &ctx(EndpointType::OpenAiResponse)).await;
        match out {
            PluginResult::Continue(b) => assert_eq!(b["prompt_cache_key"], "galaxy:ch1:key-hint"),
            _ => panic!("应 Continue"),
        }
    }

    #[test]
    fn matches_only_responses() {
        assert!(CacheKeyInjector.matches(&ctx(EndpointType::OpenAiResponse)));
        assert!(!CacheKeyInjector.matches(&ctx(EndpointType::OpenAiChat)));
        assert!(!CacheKeyInjector.matches(&ctx(EndpointType::Anthropic)));
    }
}
