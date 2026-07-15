use crate::domain::channel::EndpointType;

/// 单次尝试的统计信息。
///
/// owned by observability; relay/proxy execution only produces these facts.
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
