use crate::api::handlers::admin::channels::EndpointType;
use crate::scheduler::trace::{AttemptStatus, AttemptTrace};

/// 单次尝试的统计信息。
///
/// P13: owned by observability; relay/proxy execution only produces these facts.
pub(crate) struct AttemptStats {
    pub(crate) channel_id: String,
    pub(crate) target_model: String,
    pub(crate) upstream_endpoint: EndpointType,
    pub(crate) needs_conversion: bool,
    pub(crate) latency_ms: i64,
    pub(crate) status_code: u16,
    pub(crate) input_tokens: i32,
    pub(crate) output_tokens: i32,
    pub(crate) cache_read: i32,
    pub(crate) cache_creation: i32,
    pub(crate) cost: Option<f64>,
    pub(crate) error_message: Option<String>,
    pub(crate) upstream_key_hint: String,
}

impl AttemptStats {
    /// 转换为 scheduler::trace::AttemptTrace（M2-S1 adapter）
    /// 桥接代码：RelayRun 接入真实 proxy 后将直接使用 AttemptTrace
    #[allow(dead_code)]
    pub(crate) fn to_trace(
        &self,
        attempt_no: u32,
        requested_model: &str,
        client_endpoint: &EndpointType,
    ) -> AttemptTrace {
        AttemptTrace {
            attempt_no,
            channel_id: Some(self.channel_id.clone()),
            channel_name: None,
            upstream_key_hint: Some(self.upstream_key_hint.clone()),
            requested_model: requested_model.to_string(),
            upstream_model: Some(self.target_model.clone()),
            client_endpoint: Some(client_endpoint.as_str().to_string()),
            upstream_endpoint: Some(self.upstream_endpoint.as_str().to_string()),
            status: if (200..400).contains(&self.status_code) {
                AttemptStatus::Success
            } else {
                AttemptStatus::Failed
            },
            reason: self.error_message.clone(),
            duration_ms: Some(self.latency_ms),
            queue_wait_ms: None,
            sticky: false,
            score: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_attempt(status_code: u16) -> AttemptStats {
        AttemptStats {
            channel_id: "ch-1".into(),
            target_model: "gpt-4o".into(),
            upstream_endpoint: EndpointType::OpenAiChat,
            needs_conversion: false,
            latency_ms: 123,
            status_code,
            input_tokens: 10,
            output_tokens: 20,
            cache_read: 3,
            cache_creation: 0,
            cost: Some(0.001),
            error_message: None,
            upstream_key_hint: "sk-abcde...mnop".into(),
        }
    }

    /// 验证成功的 AttemptStats 映射到 AttemptTrace 时字段正确
    #[test]
    fn attempt_trace_log_adapter_success_maps_fields_and_serializes() {
        let stats = AttemptStats {
            channel_id: "ch-1".into(),
            target_model: "gpt-4o".into(),
            upstream_endpoint: EndpointType::OpenAiChat,
            needs_conversion: false,
            latency_ms: 150,
            status_code: 200,
            input_tokens: 10,
            output_tokens: 5,
            cache_read: 0,
            cache_creation: 0,
            cost: Some(0.001),
            error_message: None,
            upstream_key_hint: "sk-abc...xyz".into(),
        };

        let trace = stats.to_trace(1, "gpt-4o", &EndpointType::OpenAiChat);

        assert_eq!(trace.attempt_no, 1);
        assert_eq!(trace.status, AttemptStatus::Success);
        assert!(trace.reason.is_none());
        assert_eq!(trace.channel_id.as_deref(), Some("ch-1"));
        assert_eq!(trace.upstream_model.as_deref(), Some("gpt-4o"));
        assert_eq!(trace.upstream_endpoint.as_deref(), Some("openai_chat"));
        assert_eq!(trace.duration_ms, Some(150));
        assert_eq!(trace.upstream_key_hint.as_deref(), Some("sk-abc...xyz"));

        let json = serde_json::to_string(&trace).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["attempt_no"], 1);
        assert_eq!(parsed["status"], "success");
        assert_eq!(parsed["reason"], serde_json::Value::Null);
        assert_eq!(parsed["upstream_endpoint"], "openai_chat");
    }

    /// 验证失败的 AttemptStats 映射到 AttemptTrace 时 reason 和 status 正确
    #[test]
    fn attempt_trace_log_adapter_failure_maps_reason_and_status() {
        let mut stats = sample_attempt(500);
        stats.error_message = Some("upstream timeout".into());

        let trace = stats.to_trace(3, "gpt-4o", &EndpointType::OpenAiChat);

        assert_eq!(trace.attempt_no, 3);
        assert_eq!(trace.status, AttemptStatus::Failed);
        assert_eq!(trace.reason.as_deref(), Some("upstream timeout"));

        let json = serde_json::to_string(&trace).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["status"], "failed");
        assert_eq!(parsed["reason"], "upstream timeout");
    }
}
