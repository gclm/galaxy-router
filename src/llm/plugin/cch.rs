//! Claude Code 缓存标记清理（cch billing header）。
//!
//! 移植自 gt_ai_gateway `src/plugin/cchRewriter.ts`。Anthropic prompt cache 会因
//! `x-anthropic-billing-header` 中动态 cch 值失效。把 `cch=<随机>;` 归一为 `cch=A1234;`，
//! 缓存命中 0→97%。仅在 system 以 `x-anthropic-billing-header:` 开头时生效。

use std::sync::LazyLock;

use async_trait::async_trait;
use regex::Regex;
use serde_json::Value;

use super::{PluginContext, PluginResult, RequestPlugin};
use crate::domain::channel::EndpointType;

pub struct CchRewriter;

/// 移植正则 `/^\s*(x-anthropic-billing-header:[\s\S]*?cch=)[^;]+(;)/`：匹配 system
/// 开头的 billing header，捕获 `cch=` 前缀与结尾 `;`，中间随机值替换为 A1234。
static CCH_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(x-anthropic-billing-header:[\s\S]*?cch=)[^;]+(;)").expect("valid regex")
});

/// system 文本（trim 后）是否以 billing header 开头
fn starts_with_billing(s: &str) -> bool {
    s.trim().starts_with("x-anthropic-billing-header:")
}

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
        match body.get_mut("system") {
            // string 形态：整段以 billing header 开头才改
            Some(Value::String(s)) if starts_with_billing(s) => {
                let new = re.replace(s, "${1}A1234${2}");
                *s = new.into_owned();
            }
            // array 形态：仅首个 text block 且以 billing header 开头才改
            Some(Value::Array(arr)) if !arr.is_empty() => {
                let is_text = arr[0].get("type").and_then(|t| t.as_str()) == Some("text");
                let text = arr[0]
                    .get("text")
                    .and_then(|t| t.as_str())
                    .map(String::from);
                if is_text
                    && let Some(text) = text
                    && starts_with_billing(&text)
                {
                    let new = re.replace(&text, "${1}A1234${2}");
                    arr[0]["text"] = Value::String(new.into_owned());
                }
            }
            _ => {}
        }
        PluginResult::Continue(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx() -> PluginContext {
        PluginContext {
            upstream_endpoint: EndpointType::Anthropic,
            channel_id: "c".into(),
            host_key: "h".into(),
            client_name: None,
        }
    }

    #[tokio::test]
    async fn rewrites_cch_in_string_system_with_billing_header() {
        let body = json!({"system": "x-anthropic-billing-header: cc_version=2.1; cch=5a235;\nOther."});
        let out = CchRewriter.rewrite(body, &ctx()).await;
        match out {
            PluginResult::Continue(b) => {
                assert!(b["system"].as_str().unwrap().contains("cch=A1234;"));
                assert!(!b["system"].as_str().unwrap().contains("cch=5a235;"));
            }
            _ => panic!("应 Continue"),
        }
    }

    #[tokio::test]
    async fn rewrites_cch_in_array_system_first_block() {
        let body = json!({"system": [
            {"type": "text", "text": "x-anthropic-billing-header: cc_version=2.0; cch=old;\nInst1"},
            {"type": "text", "text": "Inst2"},
        ]});
        let out = CchRewriter.rewrite(body, &ctx()).await;
        match out {
            PluginResult::Continue(b) => {
                assert!(b["system"][0]["text"].as_str().unwrap().contains("cch=A1234;"));
                assert!(!b["system"][0]["text"].as_str().unwrap().contains("cch=old;"));
            }
            _ => panic!("应 Continue"),
        }
    }

    #[tokio::test]
    async fn no_rewrite_when_string_not_starting_with_billing_header() {
        let body = json!({"system": "You are helpful. x-anthropic-billing-header: cch=123;"});
        let out = CchRewriter.rewrite(body.clone(), &ctx()).await;
        match out {
            PluginResult::Continue(b) => assert_eq!(b, body), // 未改写
            _ => panic!("应 Continue"),
        }
    }

    #[tokio::test]
    async fn no_rewrite_when_array_first_block_not_text() {
        let body = json!({"system": [
            {"type": "image", "source": {}},
            {"type": "text", "text": "x-anthropic-billing-header: cch=123;"},
        ]});
        let out = CchRewriter.rewrite(body.clone(), &ctx()).await;
        match out {
            PluginResult::Continue(b) => assert_eq!(b, body),
            _ => panic!("应 Continue"),
        }
    }

    #[test]
    fn matches_only_anthropic() {
        assert!(CchRewriter.matches(&ctx()));
    }
}
