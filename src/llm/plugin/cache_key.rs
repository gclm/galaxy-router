//! OpenAI Responses `prompt_cache_key` 粘性路由注入。
//!
//! 移植自 gt_ai_gateway `src/plugin/responsesPromptCacheKeyRewriter.ts`。
//! key = `{host_key}:{client_name}`（空则 fallback `local` / `unknown`）；不覆盖已有值。

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{PluginContext, PluginResult, RequestPlugin};
use crate::api::handlers::admin::channels::EndpointType;

pub struct CacheKeyInjector;

/// 构造 cache key（移植 buildResponsesPromptCacheKey）：trim + 空值 fallback。
pub fn build_key(host_key: &str, client_name: &Option<String>) -> String {
    let host = {
        let t = host_key.trim();
        if t.is_empty() { "local" } else { t }
    };
    let client = client_name
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .unwrap_or("unknown");
    format!("{}:{}", host, client)
}

#[async_trait]
impl RequestPlugin for CacheKeyInjector {
    fn id(&self) -> &'static str {
        "cache_key_injection"
    }
    fn matches(&self, ctx: &PluginContext) -> bool {
        ctx.upstream_endpoint == EndpointType::OpenAiResponse
    }
    async fn rewrite(&self, mut body: Value, ctx: &PluginContext) -> PluginResult {
        // 不覆盖已有 prompt_cache_key
        let exists = body
            .get("prompt_cache_key")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty());
        if exists {
            return PluginResult::Continue(body);
        }
        body["prompt_cache_key"] = json!(build_key(&ctx.host_key, &ctx.client_name));
        PluginResult::Continue(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx(endpoint: EndpointType, host: &str, client: Option<&str>) -> PluginContext {
        PluginContext {
            upstream_endpoint: endpoint,
            channel_id: "c".into(),
            host_key: host.into(),
            client_name: client.map(String::from),
        }
    }

    #[test]
    fn build_key_from_host_and_client() {
        assert_eq!(build_key("abc12345", &Some("Codex".into())), "abc12345:Codex");
    }

    #[test]
    fn build_key_falls_back_when_blank() {
        assert_eq!(build_key("", &None), "local:unknown");
        assert_eq!(build_key(" host ", &Some(" client ".into())), "host:client");
    }

    #[tokio::test]
    async fn injects_key_when_missing() {
        let body = json!({"model": "gpt-4.1", "input": "hello"});
        let out = CacheKeyInjector
            .rewrite(body, &ctx(EndpointType::OpenAiResponse, "abc12345", Some("Codex")))
            .await;
        match out {
            PluginResult::Continue(b) => assert_eq!(b["prompt_cache_key"], "abc12345:Codex"),
            _ => panic!("应 Continue"),
        }
    }

    #[tokio::test]
    async fn does_not_override_existing_key() {
        let body = json!({"model": "gpt-4.1", "prompt_cache_key": "client-key"});
        let out = CacheKeyInjector
            .rewrite(body.clone(), &ctx(EndpointType::OpenAiResponse, "abc", Some("Codex")))
            .await;
        match out {
            PluginResult::Continue(b) => assert_eq!(b, body),
            _ => panic!("应 Continue"),
        }
    }

    #[test]
    fn matches_only_responses() {
        assert!(CacheKeyInjector.matches(&ctx(EndpointType::OpenAiResponse, "h", None)));
        assert!(!CacheKeyInjector.matches(&ctx(EndpointType::OpenAiChat, "h", None)));
    }
}
