use async_trait::async_trait;
use axum::http::HeaderMap;

use super::model::{LlmRequest, LlmResponse, LlmStreamResponse};
use super::stream_converter::StreamConverter;
use crate::api::handlers::admin::channels::EndpointType;

pub mod anthropic;
pub mod openai_chat;
pub mod openai_responses;

/// 入站转换器 trait
///
/// 将客户端请求转换为统一内部格式，将统一响应转换为客户端格式
#[async_trait]
pub trait Inbound: Send + Sync {
    /// 从 HTTP 请求体解析为统一请求
    async fn transform_request(
        &self,
        body: &[u8],
        headers: &HeaderMap,
    ) -> Result<LlmRequest, InboundError>;

    /// 将统一响应转换为客户端响应
    fn transform_response(&self, response: &LlmResponse) -> Result<Vec<u8>, InboundError>;

    /// 将统一流式响应转换为客户端流式事件（无状态版本）
    ///
    /// 保留用于兼容性，新代码应使用 `create_stream_converter`。
    #[allow(dead_code)]
    fn transform_stream_event(&self, event: &LlmStreamResponse) -> Result<Vec<u8>, InboundError>;

    /// 创建有状态的流式转换器（每次请求创建一个实例）
    ///
    /// 需要状态机的协议（如 Responses API、Anthropic）应覆盖此方法。
    fn create_stream_converter(&self) -> Box<dyn StreamConverter>;
}

/// 入站错误
#[derive(Debug, thiserror::Error)]
pub enum InboundError {
    #[error("解析请求失败: {0}")]
    ParseError(String),

    #[error("无效的请求: {0}")]
    InvalidRequest(String),

    #[error("转换失败: {0}")]
    TransformError(String),
}

/// 根据端点类型获取对应的入站转换器，不支持的端点返回 None
pub fn inbound_for(endpoint: &EndpointType) -> Option<&'static dyn Inbound> {
    match endpoint {
        EndpointType::OpenAiChat => Some(&openai_chat::OpenAiChatInbound),
        EndpointType::OpenAiResponse => Some(&openai_responses::OpenAiResponsesInbound),
        EndpointType::Anthropic => Some(&anthropic::AnthropicInbound),
        EndpointType::OpenAiEmbedding | EndpointType::OpenAiImages => Some(&openai_chat::OpenAiChatInbound),
        EndpointType::Gemini => None,
    }
}
