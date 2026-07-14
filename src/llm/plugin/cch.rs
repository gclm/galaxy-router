//! Claude Code 缓存标记清理（cch billing header）。
//!
//! 借鉴 GT AI Gateway `cchRewriter`。Claude Code 在 system 注入随机 cch 标记，导致
//! 每次请求 prompt hash 不同、上游缓存无法命中。清理后缓存命中率 0→97%。
//!
//! 正则模式为基于 design doc 描述的保守窄匹配，C3 用真实 CC 请求校准。

use std::sync::LazyLock;

use async_trait::async_trait;
use regex::Regex;
use serde_json::Value;

use super::{PluginContext, PluginResult, RequestPlugin, clean_system};
use crate::api::handlers::admin::channels::EndpointType;

pub struct CchRewriter;

/// cch 随机标记（保守窄匹配：`cch=<8+ 位字母数字>` → `cch=0`）。C3 校准真实格式。
static CCH_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"cch=[A-Za-z0-9_\-]{8,}").expect("valid regex"));

#[async_trait]
impl RequestPlugin for CchRewriter {
    fn id(&self) -> &'static str {
        "cch_rewrite"
    }
    fn matches(&self, ctx: &PluginContext) -> bool {
        ctx.upstream_endpoint == EndpointType::Anthropic
    }
    async fn rewrite(&self, mut body: Value, _ctx: &PluginContext) -> PluginResult {
        let re = &*CCH_PATTERN;
        clean_system(&mut body, |s| re.replace_all(s, "cch=0").to_string());
        PluginResult::Continue(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn matches_only_anthropic() {
        let ctx = PluginContext {
            upstream_endpoint: EndpointType::Anthropic,
            channel_id: "c".into(),
            host_key: "h".into(),
            client_name: None,
        };
        assert!(CchRewriter.matches(&ctx));
    }

    #[tokio::test]
    async fn cleans_cch_in_system_string() {
        let body = json!({"system": " preamble cch=abc1234567 tail"});
        let out = CchRewriter
            .rewrite(body, &PluginContext {
                upstream_endpoint: EndpointType::Anthropic,
                channel_id: "c".into(),
                host_key: "h".into(),
                client_name: None,
            })
            .await;
        match out {
            PluginResult::Continue(b) => assert_eq!(b["system"], " preamble cch=0 tail"),
            _ => panic!("应 Continue"),
        }
    }
}
