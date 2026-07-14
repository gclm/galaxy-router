//! Claude Code 隐私跟踪标记清洗。
//!
//! 借鉴 GT AI Gateway `claudeCodeTrackingRewriter`。移除 CC 注入的动态时间/时区标记，
//! 既破坏缓存又泄露用户信息。正则模式 C3 用真实 CC 请求校准。

use std::sync::LazyLock;

use async_trait::async_trait;
use regex::Regex;
use serde_json::Value;

use super::{PluginContext, PluginResult, RequestPlugin, clean_system};
use crate::api::handlers::admin::channels::EndpointType;

pub struct TrackingRemover;

/// 动态时间/时区标记（保守窄匹配，C3 校准 CC 实际格式）。命中替换为占位符。
static TRACKING_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        // ISO 时间戳（如 2026-07-14T12:00:00）
        Regex::new(r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}").expect("valid regex"),
        // UTC 偏移（如 UTC+8）
        Regex::new(r"UTC[+\-]\d{1,2}").expect("valid regex"),
    ]
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
        let patterns = &*TRACKING_PATTERNS;
        clean_system(&mut body, |s| {
            let mut s = s.to_string();
            for re in patterns {
                s = re.replace_all(&s, "<time>").to_string();
            }
            s
        });
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
        assert!(TrackingRemover.matches(&ctx));
    }

    #[tokio::test]
    async fn cleans_time_and_timezone_in_system() {
        let body = json!({"system": "now 2026-07-14T12:00:00 UTC+8 done"});
        let out = TrackingRemover
            .rewrite(
                body,
                &PluginContext {
                    upstream_endpoint: EndpointType::Anthropic,
                    channel_id: "c".into(),
                    host_key: "h".into(),
                    client_name: None,
                },
            )
            .await;
        match out {
            PluginResult::Continue(b) => {
                let s = b["system"].as_str().unwrap();
                assert!(!s.contains("2026-07-14T12:00:00"));
                assert!(!s.contains("UTC+8"));
                assert!(s.contains("<time>"));
            }
            _ => panic!("应 Continue"),
        }
    }
}
