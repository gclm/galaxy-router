use crate::domain::channel::EndpointType;
use crate::service::pricing::model::ModelRegistry;
use crate::llm::relay::prepare::{
    estimate_tokens, extract_request_text, extract_response_text, extract_usage,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UsageSnapshot {
    pub(crate) input_tokens: i32,
    pub(crate) output_tokens: i32,
    pub(crate) cache_read: i32,
    pub(crate) cache_creation: i32,
}

impl UsageSnapshot {
    pub(crate) fn new(
        input_tokens: i32,
        output_tokens: i32,
        cache_read: i32,
        cache_creation: i32,
    ) -> Self {
        Self {
            input_tokens,
            output_tokens,
            cache_read,
            cache_creation,
        }
    }
}

pub(crate) fn resolve_non_stream_usage(
    request_body: &serde_json::Value,
    response_body: &serde_json::Value,
    upstream_endpoint: &EndpointType,
    status_code: u16,
) -> UsageSnapshot {
    if !(200..400).contains(&status_code) {
        return UsageSnapshot::new(0, 0, 0, 0);
    }

    let (input_tokens, output_tokens, cache_read, cache_creation) =
        extract_usage(response_body, upstream_endpoint);
    if input_tokens == 0 && output_tokens == 0 {
        let req_text = extract_request_text(request_body);
        let resp_text = extract_response_text(response_body);
        UsageSnapshot::new(
            estimate_tokens(&req_text),
            estimate_tokens(&resp_text),
            cache_read,
            cache_creation,
        )
    } else {
        UsageSnapshot::new(input_tokens, output_tokens, cache_read, cache_creation)
    }
}

pub(crate) fn resolve_stream_usage(
    upstream_endpoint: &EndpointType,
    last_usage: Option<serde_json::Value>,
    input_usage: Option<serde_json::Value>,
    request_text_for_estimation: &str,
    collected_text: &str,
) -> UsageSnapshot {
    let (mut input_tokens, mut output_tokens, cache_read, cache_creation) = match upstream_endpoint
    {
        EndpointType::Anthropic => {
            let input = input_usage
                .as_ref()
                .and_then(|u| u["input_tokens"].as_i64())
                .filter(|&v| v > 0)
                .or_else(|| {
                    last_usage
                        .as_ref()
                        .and_then(|u| u["usage"]["input_tokens"].as_i64())
                })
                .unwrap_or(0) as i32;
            let output = last_usage
                .as_ref()
                .and_then(|u| u["usage"]["output_tokens"].as_i64())
                .unwrap_or(0) as i32;
            let cache_read = input_usage
                .as_ref()
                .and_then(|u| u["cache_read_input_tokens"].as_i64())
                .filter(|&v| v > 0)
                .or_else(|| {
                    last_usage
                        .as_ref()
                        .and_then(|u| u["usage"]["cache_read_input_tokens"].as_i64())
                })
                .unwrap_or(0) as i32;
            let cache_creation = input_usage
                .as_ref()
                .and_then(|u| u["cache_creation_input_tokens"].as_i64())
                .filter(|&v| v > 0)
                .or_else(|| {
                    last_usage
                        .as_ref()
                        .and_then(|u| u["usage"]["cache_creation_input_tokens"].as_i64())
                })
                .unwrap_or(0) as i32;
            (input, output, cache_read, cache_creation)
        }
        _ => last_usage
            .map(|u| extract_usage(&u, upstream_endpoint))
            .unwrap_or((0, 0, 0, 0)),
    };

    if input_tokens == 0 && output_tokens == 0 {
        input_tokens = estimate_tokens(request_text_for_estimation);
        output_tokens = estimate_tokens(collected_text);
    }

    UsageSnapshot::new(input_tokens, output_tokens, cache_read, cache_creation)
}

pub(crate) async fn calculate_cost(
    model_registry: &ModelRegistry,
    target_model: &str,
    usage: UsageSnapshot,
) -> Option<f64> {
    if usage.input_tokens > 0 || usage.output_tokens > 0 {
        Some(
            model_registry
                .calculate_cost(
                    target_model,
                    usage.input_tokens,
                    usage.output_tokens,
                    usage.cache_read,
                    usage.cache_creation,
                )
                .await,
        )
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_usage_anthropic_prefers_input_usage_for_cache_tokens() {
        let usage = resolve_stream_usage(
            &EndpointType::Anthropic,
            Some(serde_json::json!({"usage":{"input_tokens": 1, "output_tokens": 7}})),
            Some(serde_json::json!({
                "input_tokens": 10,
                "cache_read_input_tokens": 2,
                "cache_creation_input_tokens": 3
            })),
            "hello",
            "world",
        );
        assert_eq!(usage, UsageSnapshot::new(10, 7, 2, 3));
    }

    #[test]
    fn stream_usage_falls_back_to_estimation_when_missing_usage() {
        let usage =
            resolve_stream_usage(&EndpointType::OpenAiChat, None, None, "hello world", "ok");
        assert!(usage.input_tokens > 0);
        assert!(usage.output_tokens > 0);
    }

    #[test]
    fn non_stream_usage_returns_zero_for_error_status() {
        let usage = resolve_non_stream_usage(
            &serde_json::json!({"messages": []}),
            &serde_json::json!({"error": "bad"}),
            &EndpointType::OpenAiChat,
            500,
        );
        assert_eq!(usage, UsageSnapshot::new(0, 0, 0, 0));
    }
}
