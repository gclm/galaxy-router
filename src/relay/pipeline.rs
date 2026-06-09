use axum::http::HeaderMap;

use crate::api::handlers::admin::channels::EndpointType;
use crate::protocol::inbound::{Inbound, InboundError};
use crate::protocol::outbound::{Outbound, OutboundError};
use crate::protocol::stream_converter::StreamConverter;

#[derive(Debug, Clone, PartialEq)]
pub struct RelayPipelineRequest {
    pub client_endpoint: EndpointType,
    pub upstream_endpoint: EndpointType,
    pub requested_model: String,
    pub upstream_model: String,
    pub body: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreparedRelayRequest {
    pub client_endpoint: EndpointType,
    pub upstream_endpoint: EndpointType,
    pub requested_model: String,
    pub upstream_model: String,
    pub body: serde_json::Value,
    pub needs_conversion: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FinalizedRelayResponse {
    pub body: serde_json::Value,
    pub was_converted: bool,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RelayPipelineError {
    #[error("unsupported relay pipeline endpoint: {endpoint:?}")]
    UnsupportedEndpoint { endpoint: EndpointType },

    #[error("inbound conversion failed: {0}")]
    Inbound(String),

    #[error("outbound conversion failed: {0}")]
    Outbound(String),

    #[error("json conversion failed: {0}")]
    Json(String),
}

impl From<InboundError> for RelayPipelineError {
    fn from(value: InboundError) -> Self {
        Self::Inbound(value.to_string())
    }
}

impl From<OutboundError> for RelayPipelineError {
    fn from(value: OutboundError) -> Self {
        Self::Outbound(value.to_string())
    }
}

pub struct RelayPipeline;

impl RelayPipeline {
    pub fn prepare_request(
        request: RelayPipelineRequest,
    ) -> Result<PreparedRelayRequest, RelayPipelineError> {
        if request.client_endpoint != request.upstream_endpoint {
            return Err(RelayPipelineError::UnsupportedEndpoint {
                endpoint: request.upstream_endpoint,
            });
        }

        let mut body = request.body;
        rewrite_model(&mut body, &request.upstream_model);

        Ok(PreparedRelayRequest {
            client_endpoint: request.client_endpoint,
            upstream_endpoint: request.upstream_endpoint,
            requested_model: request.requested_model,
            upstream_model: request.upstream_model,
            body,
            needs_conversion: false,
        })
    }

    pub async fn prepare_request_async(
        request: RelayPipelineRequest,
    ) -> Result<PreparedRelayRequest, RelayPipelineError> {
        if request.client_endpoint == request.upstream_endpoint {
            return Self::prepare_request(request);
        }

        let body_bytes = serde_json::to_vec(&request.body)
            .map_err(|e| RelayPipelineError::Json(e.to_string()))?;
        let mut llm_request = inbound_for(&request.client_endpoint)?
            .transform_request(&body_bytes, &HeaderMap::new())
            .await?;
        llm_request.model = request.upstream_model.clone();

        let upstream_body_bytes =
            outbound_for(&request.upstream_endpoint)?.transform_request(&llm_request)?;
        let upstream_body = serde_json::from_slice(&upstream_body_bytes)
            .map_err(|e| RelayPipelineError::Json(e.to_string()))?;

        Ok(PreparedRelayRequest {
            client_endpoint: request.client_endpoint,
            upstream_endpoint: request.upstream_endpoint,
            requested_model: request.requested_model,
            upstream_model: request.upstream_model,
            body: upstream_body,
            needs_conversion: true,
        })
    }

    pub fn finalize_response(
        client_endpoint: EndpointType,
        upstream_endpoint: EndpointType,
        body: serde_json::Value,
    ) -> Result<FinalizedRelayResponse, RelayPipelineError> {
        if client_endpoint != upstream_endpoint {
            return Err(RelayPipelineError::UnsupportedEndpoint {
                endpoint: upstream_endpoint,
            });
        }

        Ok(FinalizedRelayResponse {
            body,
            was_converted: false,
        })
    }

    pub async fn finalize_response_async(
        client_endpoint: EndpointType,
        upstream_endpoint: EndpointType,
        body: serde_json::Value,
        status: u16,
    ) -> Result<FinalizedRelayResponse, RelayPipelineError> {
        if client_endpoint == upstream_endpoint {
            return Self::finalize_response(client_endpoint, upstream_endpoint, body);
        }

        let body_bytes =
            serde_json::to_vec(&body).map_err(|e| RelayPipelineError::Json(e.to_string()))?;
        let llm_response = outbound_for(&upstream_endpoint)?
            .transform_response(&body_bytes, status)
            .await?;
        let client_body_bytes = inbound_for(&client_endpoint)?.transform_response(&llm_response)?;
        let client_body = serde_json::from_slice(&client_body_bytes)
            .map_err(|e| RelayPipelineError::Json(e.to_string()))?;

        Ok(FinalizedRelayResponse {
            body: client_body,
            was_converted: true,
        })
    }

    /// 流式转换是否需要跨协议转换
    #[allow(dead_code)]
    pub fn needs_stream_conversion(
        client_endpoint: &EndpointType,
        upstream_endpoint: &EndpointType,
    ) -> bool {
        client_endpoint != upstream_endpoint
    }

    /// 创建客户端协议的流式转换器（conversion 路径使用）
    ///
    /// passthrough 路径（相同端点）返回 None
    pub fn create_stream_converter(
        client_endpoint: &EndpointType,
        upstream_endpoint: &EndpointType,
    ) -> Result<Option<Box<dyn StreamConverter>>, RelayPipelineError> {
        if client_endpoint == upstream_endpoint {
            return Ok(None);
        }
        let converter = inbound_for(client_endpoint)?.create_stream_converter();
        Ok(Some(converter))
    }

    /// 解码上游 SSE 事件为统一的 LlmStreamResponse
    ///
    /// 返回 None 表示事件被忽略（如心跳/注释行）
    pub fn decode_stream_event(
        upstream_endpoint: &EndpointType,
        event_bytes: &[u8],
    ) -> Result<Option<crate::protocol::model::LlmStreamResponse>, RelayPipelineError> {
        let result = outbound_for(upstream_endpoint)?
            .transform_stream_event(event_bytes)
            .map_err(|e| RelayPipelineError::Outbound(e.to_string()))?;
        Ok(result)
    }
}

fn rewrite_model(body: &mut serde_json::Value, upstream_model: &str) {
    if let Some(obj) = body.as_object_mut() {
        obj.insert(
            "model".to_string(),
            serde_json::Value::String(upstream_model.to_string()),
        );
    }
}

fn inbound_for(endpoint: &EndpointType) -> Result<&'static dyn Inbound, RelayPipelineError> {
    crate::protocol::inbound::inbound_for(endpoint).ok_or(RelayPipelineError::UnsupportedEndpoint {
        endpoint: endpoint.clone(),
    })
}

fn outbound_for(endpoint: &EndpointType) -> Result<&'static dyn Outbound, RelayPipelineError> {
    crate::protocol::outbound::outbound_for(endpoint).ok_or(
        RelayPipelineError::UnsupportedEndpoint {
            endpoint: endpoint.clone(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// M1-S2 characterization: passthrough 直通路径保持响应体原样
    #[test]
    fn response_pipeline_conversion_passthrough_preserves_body() {
        let upstream_body = serde_json::json!({
            "id": "chatcmpl-42",
            "object": "chat.completion",
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "hello"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7}
        });

        let finalized = RelayPipeline::finalize_response(
            EndpointType::OpenAiChat,
            EndpointType::OpenAiChat,
            upstream_body.clone(),
        )
        .expect("passthrough finalize should succeed");

        assert!(
            !finalized.was_converted,
            "passthrough should not mark as converted"
        );
        assert_eq!(
            finalized.body, upstream_body,
            "passthrough body must be identical"
        );
    }

    /// M1-S2 characterization: 转换路径将 Anthropic 响应转为 OpenAI Chat 格式
    #[tokio::test]
    async fn response_pipeline_conversion_anthropic_to_openai_chat() {
        let anthropic_response = serde_json::json!({
            "id": "msg_conv",
            "type": "message",
            "role": "assistant",
            "model": "claude-3-5-sonnet",
            "content": [{"type": "text", "text": "converted reply"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 8, "output_tokens": 3}
        });

        let finalized = RelayPipeline::finalize_response_async(
            EndpointType::OpenAiChat,
            EndpointType::Anthropic,
            anthropic_response,
            200,
        )
        .await
        .expect("conversion finalize should succeed");

        assert!(
            finalized.was_converted,
            "cross-protocol should mark as converted"
        );
        assert_eq!(finalized.body["object"], "chat.completion");
        assert_eq!(finalized.body["model"], "claude-3-5-sonnet");
        assert_eq!(
            finalized.body["choices"][0]["message"]["content"],
            "converted reply"
        );
        assert_eq!(finalized.body["choices"][0]["finish_reason"], "stop");
        assert_eq!(finalized.body["usage"]["prompt_tokens"], 8);
        assert_eq!(finalized.body["usage"]["completion_tokens"], 3);
        assert_eq!(finalized.body["usage"]["total_tokens"], 11);
    }

    /// M1-S2 characterization: 转换路径将 OpenAI Responses 响应转为 OpenAI Chat 格式
    #[tokio::test]
    async fn response_pipeline_conversion_responses_to_chat() {
        let responses_body = serde_json::json!({
            "id": "resp_conv",
            "object": "response",
            "model": "gpt-4o",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "hi from responses"}]
            }],
            "usage": {"input_tokens": 4, "output_tokens": 2, "total_tokens": 6}
        });

        let finalized = RelayPipeline::finalize_response_async(
            EndpointType::OpenAiChat,
            EndpointType::OpenAiResponse,
            responses_body,
            200,
        )
        .await
        .expect("responses→chat conversion should succeed");

        assert!(finalized.was_converted);
        assert_eq!(finalized.body["object"], "chat.completion");
        assert_eq!(
            finalized.body["choices"][0]["message"]["content"],
            "hi from responses"
        );
        assert_eq!(finalized.body["usage"]["prompt_tokens"], 4);
        assert_eq!(finalized.body["usage"]["completion_tokens"], 2);
    }

    /// M1-S2 characterization: 相同端点 finalize_response_async 走 passthrough
    #[tokio::test]
    async fn response_pipeline_conversion_same_endpoint_uses_fast_path() {
        let body = serde_json::json!({"id": "test", "result": "ok"});
        let finalized = RelayPipeline::finalize_response_async(
            EndpointType::Anthropic,
            EndpointType::Anthropic,
            body.clone(),
            200,
        )
        .await
        .expect("same-endpoint async finalize should succeed");

        assert!(!finalized.was_converted);
        assert_eq!(finalized.body, body);
    }

    // ============================================================
    // P4.4a: Stream pipeline characterization
    // ============================================================

    #[test]
    fn stream_pipeline_needs_conversion_true_for_different_endpoints() {
        assert!(RelayPipeline::needs_stream_conversion(
            &EndpointType::OpenAiChat,
            &EndpointType::Anthropic
        ));
    }

    #[test]
    fn stream_pipeline_needs_conversion_false_for_same_endpoint() {
        assert!(!RelayPipeline::needs_stream_conversion(
            &EndpointType::OpenAiChat,
            &EndpointType::OpenAiChat
        ));
    }

    #[test]
    fn stream_pipeline_create_converter_returns_none_for_passthrough() {
        let converter = RelayPipeline::create_stream_converter(
            &EndpointType::OpenAiChat,
            &EndpointType::OpenAiChat,
        )
        .expect("passthrough should succeed");
        assert!(converter.is_none());
    }

    #[test]
    fn stream_pipeline_create_converter_returns_some_for_conversion() {
        let converter = RelayPipeline::create_stream_converter(
            &EndpointType::OpenAiChat,
            &EndpointType::Anthropic,
        )
        .expect("anthropic→chat should succeed");
        assert!(converter.is_some());
    }

    /// 验证 decode_stream_event 能解析 Anthropic content_block_delta 事件
    #[test]
    fn stream_pipeline_decode_anthropic_content_delta() {
        // Anthropic SSE event: content_block_delta with text
        let event = b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\n";

        let llm_event = RelayPipeline::decode_stream_event(&EndpointType::Anthropic, event)
            .expect("decode should succeed")
            .expect("should produce LlmStreamResponse");

        // 验证解码后的 canonical 事件包含文本
        if let Some(choice) = llm_event.first_choice() {
            if let Some(crate::protocol::model::Content::Text(t)) = &choice.delta.content {
                assert_eq!(t, "Hi");
            } else {
                panic!("expected text content in delta");
            }
        } else {
            panic!("expected first_choice in decoded event");
        }
    }
}
