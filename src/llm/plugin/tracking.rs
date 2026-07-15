//! Claude Code 隐私跟踪标记清洗。
//!
//! 移植自 gt_ai_gateway `src/plugin/claudeCodeTrackingRewriter.ts`。Claude Code 在
//! system 注入 `# currentDate\nToday's date is YYYY/MM/DD.` 块（含 Unicode 引号变体 'ʼʹ
//! 与 `/` 分隔符），每次请求微变 → prompt cache 失效。归一为标准引号 `'` + `-` 分隔。

use std::sync::LazyLock;

use async_trait::async_trait;
use regex::Regex;
use serde_json::Value;

use super::{PluginContext, PluginResult, RequestPlugin, clean_system};
use crate::domain::channel::EndpointType;

pub struct TrackingRemover;

/// 移植正则 `/(# currentDate\r?\n)Today(?:'|’|ʼ|ʹ)s date is
/// (\d{4})[/-](\d{2})[/-](\d{2})\.(\r?\n)/g`：仅在 `# currentDate` 块内匹配，
/// 归一引号与分隔符。
static TRACKING_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(# currentDate\r?\n)Today(?:'|\u{2019}|\u{02BC}|\u{02B9})s date is (\d{4})[/-](\d{2})[/-](\d{2})\.(\r?\n)",
    )
    .expect("valid regex")
});

#[async_trait]
impl RequestPlugin for TrackingRemover {
    fn id(&self) -> &'static str {
        "tracking_removal"
    }
    fn matches(&self, ctx: &PluginContext) -> bool {
        ctx.upstream_endpoint == EndpointType::Anthropic
    }
    async fn rewrite(&self, mut body: Value, _ctx: &PluginContext) -> PluginResult {
        let re = &*TRACKING_PATTERN;
        clean_system(&mut body, |s| {
            re.replace_all(s, "${1}Today's date is ${2}-${3}-${4}.${5}")
                .into_owned()
        });
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
    async fn normalizes_slashed_date_and_apostrophe_in_block() {
        let body = json!({"system": "# currentDate\nToday's date is 2026/06/30.\n\nOther."});
        let out = TrackingRemover.rewrite(body, &ctx()).await;
        match out {
            PluginResult::Continue(b) => {
                let s = b["system"].as_str().unwrap();
                assert!(s.contains("Today's date is 2026-06-30."));
                assert!(!s.contains("2026/06/30"));
            }
            _ => panic!("应 Continue"),
        }
    }

    #[tokio::test]
    async fn normalizes_unicode_apostrophe_variants() {
        for apo in ["\u{2019}", "\u{02BC}", "\u{02B9}"] {
            let body = json!({"system": format!("# currentDate\nToday{apo}s date is 2026/06/30.\n")});
            let out = TrackingRemover.rewrite(body, &ctx()).await;
            match out {
                PluginResult::Continue(b) => {
                    assert_eq!(b["system"], "# currentDate\nToday's date is 2026-06-30.\n");
                }
                _ => panic!("应 Continue"),
            }
        }
    }

    #[tokio::test]
    async fn normalizes_in_array_system() {
        let body = json!({"system": [
            {"type": "text", "text": "Inst1"},
            {"type": "text", "text": "prefix\n# currentDate\nToday\u{2019}s date is 2026/06/30.\ntail"},
        ]});
        let out = TrackingRemover.rewrite(body, &ctx()).await;
        match out {
            PluginResult::Continue(b) => {
                let s = b["system"][1]["text"].as_str().unwrap();
                assert!(s.contains("Today's date is 2026-06-30."));
                assert!(!s.contains("2026/06/30"));
            }
            _ => panic!("应 Continue"),
        }
    }

    #[tokio::test]
    async fn no_rewrite_when_marker_outside_block() {
        let body = json!({"system": "The user said: Today's date is 2026/06/30. Hm?"});
        let out = TrackingRemover.rewrite(body.clone(), &ctx()).await;
        match out {
            PluginResult::Continue(b) => assert_eq!(b, body),
            _ => panic!("应 Continue"),
        }
    }

    #[test]
    fn matches_only_anthropic() {
        assert!(TrackingRemover.matches(&ctx()));
    }
}
