use async_trait::async_trait;

use super::model::{LlmRequest, LlmResponse, LlmStreamResponse};

/// 出站转换器 trait
///
/// 将统一请求转换为提供商格式，将提供商响应转换为统一格式
#[async_trait]
pub trait Outbound: Send + Sync {
    /// 将统一请求转换为提供商请求
    fn transform_request(&self, request: &LlmRequest) -> Result<Vec<u8>, OutboundError>;

    /// 将提供商响应转换为统一响应
    async fn transform_response(
        &self,
        body: &[u8],
        status: u16,
    ) -> Result<LlmResponse, OutboundError>;

    /// 将提供商流式事件转换为统一流式响应
    fn transform_stream_event(
        &self,
        event: &[u8],
    ) -> Result<Option<LlmStreamResponse>, OutboundError>;

    /// 设置认证头
    fn set_auth_header(&self, headers: &mut reqwest::header::HeaderMap, api_key: &str);
}

/// 出站错误
#[derive(Debug, thiserror::Error)]
#[allow(clippy::enum_variant_names)]
pub enum OutboundError {
    #[error("转换请求失败: {0}")]
    TransformError(String),

    #[error("解析响应失败: {0}")]
    ParseError(String),
}
