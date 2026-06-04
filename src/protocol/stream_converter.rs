use thiserror::Error;

use super::model::LlmStreamResponse;

/// 流式转换错误
#[derive(Debug, Error)]
pub enum StreamConvertError {
    #[error("转换失败: {0}")]
    ConvertError(String),
}

/// 有状态的流式转换器（每次请求创建一个实例）
///
/// 将统一的 `LlmStreamResponse` 流事件转换为客户端协议的 SSE 事件。
/// 一个输入事件可能产生 0..N 个输出事件（例如 Responses API 需要发送
/// `output_item.added` + `content_part.added` + delta 等多个事件）。
pub trait StreamConverter: Send {
    /// 将一个统一流事件转换为 0..N 个客户端 SSE 事件
    ///
    /// 每个返回的 `Vec<u8>` 是一个完整的 SSE 事件（`event: ...\ndata: ...\n\n`）
    fn convert(&mut self, event: &LlmStreamResponse) -> Result<Vec<Vec<u8>>, StreamConvertError>;

    /// 流结束时调用，返回关闭事件（如 `response.completed`、`message_stop` 等）
    fn finish(&mut self) -> Result<Vec<Vec<u8>>, StreamConvertError>;
}

/// 简单的无状态流式转换器包装
///
/// 将现有的无状态 `transform_stream_event` 闭包包装为 `StreamConverter` 接口。
/// 用于不需要状态机的协议（如 OpenAI Chat Completions）。
pub struct SimpleStreamConverter<F>
where
    F: Fn(&LlmStreamResponse) -> Result<Vec<u8>, super::inbound::InboundError> + Send,
{
    transform_fn: F,
}

impl<F> SimpleStreamConverter<F>
where
    F: Fn(&LlmStreamResponse) -> Result<Vec<u8>, super::inbound::InboundError> + Send,
{
    pub fn new(transform_fn: F) -> Self {
        Self { transform_fn }
    }
}

impl<F> StreamConverter for SimpleStreamConverter<F>
where
    F: Fn(&LlmStreamResponse) -> Result<Vec<u8>, super::inbound::InboundError> + Send,
{
    fn convert(&mut self, event: &LlmStreamResponse) -> Result<Vec<Vec<u8>>, StreamConvertError> {
        match (self.transform_fn)(event) {
            Ok(data) => {
                if data.is_empty() {
                    Ok(vec![])
                } else {
                    Ok(vec![data])
                }
            }
            Err(e) => Err(StreamConvertError::ConvertError(e.to_string())),
        }
    }

    fn finish(&mut self) -> Result<Vec<Vec<u8>>, StreamConvertError> {
        // 无状态转换器不需要关闭事件
        Ok(vec![])
    }
}
