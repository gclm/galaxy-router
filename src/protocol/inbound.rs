use async_trait::async_trait;
use axum::http::HeaderMap;

use super::model::{LlmRequest, LlmResponse, LlmStreamResponse};

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

    /// 将统一流式响应转换为客户端流式事件
    fn transform_stream_event(&self, event: &LlmStreamResponse) -> Result<Vec<u8>, InboundError>;
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
