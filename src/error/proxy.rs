use axum::http::StatusCode;

use crate::protocol::sse::sanitize_upstream_error;

/// 错误格式类型
pub enum ErrorFormat {
    /// OpenAI 格式: {"error": {"message": ..., "type": ...}}
    OpenAi,
    /// Anthropic 格式: {"type": "error", "error": {"type": ..., "message": ...}}
    Anthropic,
}

/// 代理错误
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ProxyError {
    #[error("数据库错误: {0}")]
    DatabaseError(String),

    #[error("渠道不存在: {0}")]
    ChannelNotFound(String),

    #[error("没有可用渠道: {0}")]
    NoAvailableChannel(String),

    #[error("模型不支持: {0}")]
    ModelNotSupported(String),

    #[error("模型不存在: {0}")]
    ModelNotFound(String),

    #[error("请求失败: {0}")]
    RequestError(String),

    #[error("转换失败: {0}")]
    TransformError(String),

    #[error("上游错误: {status}")]
    UpstreamError { status: StatusCode, body: String },
}

/// 错误分类（用于重试与降级策略）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// 上游 Key 相关问题（401、402、429、余额不足、无效 key 等）— 换 Key
    KeyRetryable,
    /// 上游临时错误（5xx、超时）— 可换渠道
    UpstreamRetryable,
    /// 客户端错误（4xx、非 Key 鉴权）— 不重试
    Client,
    /// 系统/内部错误 — 不重试
    Internal,
}

impl ProxyError {
    /// 错误分类
    pub fn classify(&self) -> ErrorClass {
        match self {
            ProxyError::DatabaseError(_) | ProxyError::ChannelNotFound(_) => ErrorClass::Internal,
            ProxyError::NoAvailableChannel(_) | ProxyError::RequestError(_) => {
                ErrorClass::UpstreamRetryable
            }
            ProxyError::ModelNotSupported(_) | ProxyError::ModelNotFound(_) => ErrorClass::Client,
            ProxyError::TransformError(_) => ErrorClass::Internal,
            ProxyError::UpstreamError { status, body } => classify_upstream(*status, body),
        }
    }

    /// 上游错误是否应当换 Key 重试
    pub fn is_key_retryable(&self) -> bool {
        matches!(self.classify(), ErrorClass::KeyRetryable)
    }

    /// 从 RelayRunOutcome 构建错误
    pub fn from_relay_outcome(outcome: &crate::relay::run::RelayRunOutcome) -> Self {
        match outcome.status_code {
            404 => ProxyError::ModelNotFound(
                outcome
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "模型不存在".to_string()),
            ),
            _ => {
                if let Some(ref msg) = outcome.error_message {
                    if outcome.status_code >= 500 {
                        ProxyError::UpstreamError {
                            status: StatusCode::from_u16(outcome.status_code)
                                .unwrap_or(StatusCode::BAD_GATEWAY),
                            body: msg.clone(),
                        }
                    } else {
                        ProxyError::NoAvailableChannel(msg.clone())
                    }
                } else {
                    ProxyError::NoAvailableChannel("所有渠道都不可用".to_string())
                }
            }
        }
    }

    /// 从 RelayStreamRunOutcome 构建错误
    pub fn from_relay_stream_outcome(outcome: &crate::relay::run::RelayStreamRunOutcome) -> Self {
        match outcome.status_code {
            404 => ProxyError::ModelNotFound(
                outcome
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "模型不存在".to_string()),
            ),
            _ => {
                if let Some(ref msg) = outcome.error_message {
                    if outcome.status_code >= 500 {
                        ProxyError::UpstreamError {
                            status: StatusCode::from_u16(outcome.status_code)
                                .unwrap_or(StatusCode::BAD_GATEWAY),
                            body: msg.clone(),
                        }
                    } else {
                        ProxyError::NoAvailableChannel(msg.clone())
                    }
                } else {
                    ProxyError::NoAvailableChannel("所有渠道都不可用".to_string())
                }
            }
        }
    }
}

fn classify_upstream(status: StatusCode, body: &str) -> ErrorClass {
    // 429 / 503 都是上游"瞬时过载"类错误，应优先尝试换 key（参考 sub2api 对 503 的特殊处理）
    if matches!(
        status,
        StatusCode::UNAUTHORIZED
            | StatusCode::PAYMENT_REQUIRED
            | StatusCode::TOO_MANY_REQUESTS
            | StatusCode::SERVICE_UNAVAILABLE
    ) {
        return ErrorClass::KeyRetryable;
    }

    let lower = sanitize_upstream_error(body).to_ascii_lowercase();
    const KEY_NEEDLES: &[&str] = &[
        "余额不足",
        "无可用资源包",
        "速率限制",
        "频率限制",
        "insufficient_quota",
        "quota exceeded",
        "resource exhausted",
        "credit balance",
        "billing",
        "rate limit",
        "invalid api key",
        "incorrect api key",
        "unauthorized",
        "authentication",
    ];
    if KEY_NEEDLES.iter().any(|n| lower.contains(n)) {
        return ErrorClass::KeyRetryable;
    }

    if status.is_server_error() {
        ErrorClass::UpstreamRetryable
    } else {
        ErrorClass::Client
    }
}

/// 格式化 402 预算超限错误
pub fn format_budget_error(msg: &str, error_format: &ErrorFormat) -> axum::response::Response {
    use axum::response::IntoResponse;

    let body = match error_format {
        ErrorFormat::OpenAi => serde_json::json!({
            "error": {
                "message": msg,
                "type": "insufficient_quota",
                "code": "budget_exceeded"
            }
        }),
        ErrorFormat::Anthropic => serde_json::json!({
            "type": "error",
            "error": { "type": "insufficient_quota", "message": msg }
        }),
    };

    (StatusCode::PAYMENT_REQUIRED, axum::Json(body)).into_response()
}

/// 格式化代理错误为 HTTP 响应
pub fn format_proxy_error(e: ProxyError, format: &ErrorFormat) -> axum::response::Response {
    use axum::response::IntoResponse;

    let (status, message) = render_status_and_message(&e);
    let body = match format {
        ErrorFormat::OpenAi => serde_json::json!({
            "error": { "message": message, "type": "server_error" }
        }),
        ErrorFormat::Anthropic => serde_json::json!({
            "type": "error",
            "error": { "type": "api_error", "message": message }
        }),
    };
    (status, axum::Json(body)).into_response()
}

/// 格式化 429 速率限制错误
pub fn format_rate_limit_error(
    retry_after: f64,
    limit_type: &str,
    limit: u64,
    error_format: &ErrorFormat,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let message = format!(
        "速率限制 exceeded: {} limit={}，请 {:.0} 秒后重试",
        limit_type, limit, retry_after
    );
    let body = match error_format {
        ErrorFormat::OpenAi => serde_json::json!({
            "error": {
                "message": message,
                "type": "rate_limit_error",
                "code": "rate_limit_exceeded"
            }
        }),
        ErrorFormat::Anthropic => serde_json::json!({
            "type": "error",
            "error": { "type": "rate_limit_error", "message": message }
        }),
    };

    (
        StatusCode::TOO_MANY_REQUESTS,
        [
            ("Retry-After", format!("{:.0}", retry_after)),
            ("X-RateLimit-Limit", limit.to_string()),
        ],
        axum::Json(body),
    )
        .into_response()
}

/// 提取 (HTTP 状态码, 客户端可见消息) — 与客户端格式无关
fn render_status_and_message(e: &ProxyError) -> (StatusCode, String) {
    match e {
        ProxyError::NoAvailableChannel(msg) => (StatusCode::SERVICE_UNAVAILABLE, msg.clone()),
        ProxyError::ModelNotSupported(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
        ProxyError::ModelNotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
        ProxyError::DatabaseError(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
        ProxyError::ChannelNotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
        ProxyError::RequestError(msg) => (StatusCode::BAD_GATEWAY, msg.clone()),
        ProxyError::TransformError(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
        ProxyError::UpstreamError { status, body } => (*status, sanitize_upstream_error(body)),
    }
}

/// 兼容封装：仅根据 status + body 判断上游错误是否需要换 Key
///
/// 内部委托给 `ProxyError::is_key_retryable`，避免散落的多处独立判断。
pub fn is_key_retryable_upstream_error(status: StatusCode, body: &str) -> bool {
    ProxyError::UpstreamError {
        status,
        body: body.to_string(),
    }
    .is_key_retryable()
}

/// SSE 流内错误的 HTTP 状态归因
///
/// 上游返回 2xx 但 SSE 流内含错误时（如智谱 1302 限流藏在流里），根据错误体归因
/// 状态码：限流/鉴权语义 → 429（触发换 key、不触发 channel 黑名单），其他保持 502。
pub fn sse_stream_error_status(body: &str) -> StatusCode {
    if is_key_retryable_upstream_error(StatusCode::BAD_GATEWAY, body) {
        StatusCode::TOO_MANY_REQUESTS
    } else {
        StatusCode::BAD_GATEWAY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_retryable_error_matches_status_and_quota_body() {
        assert!(is_key_retryable_upstream_error(
            StatusCode::UNAUTHORIZED,
            r#"{"error":{"message":"bad key"}}"#
        ));
        assert!(is_key_retryable_upstream_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"error":{"message":"insufficient_quota"}}"#
        ));
        assert!(is_key_retryable_upstream_error(
            StatusCode::BAD_REQUEST,
            r#"{"error":{"message":"余额不足或无可用资源包"}}"#
        ));
    }

    #[test]
    fn key_retryable_error_ignores_non_key_errors() {
        assert!(!is_key_retryable_upstream_error(
            StatusCode::BAD_REQUEST,
            r#"{"error":{"message":"model does not exist"}}"#
        ));
        assert!(!is_key_retryable_upstream_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"error":{"message":"upstream overloaded"}}"#
        ));
    }

    #[test]
    fn classify_upstream_distinguishes_key_retryable() {
        // Key 相关：401 / 402 / 429 / 503
        assert_eq!(
            classify_upstream(StatusCode::UNAUTHORIZED, ""),
            ErrorClass::KeyRetryable
        );
        assert_eq!(
            classify_upstream(StatusCode::TOO_MANY_REQUESTS, ""),
            ErrorClass::KeyRetryable
        );
        // 503：上游瞬时过载（如 mimo "Service temporarily unavailable"），优先换 key，不进渠道黑名单
        assert_eq!(
            classify_upstream(StatusCode::SERVICE_UNAVAILABLE, ""),
            ErrorClass::KeyRetryable
        );
        // 5xx 非 Key 字样：可换渠道重试
        assert_eq!(
            classify_upstream(StatusCode::INTERNAL_SERVER_ERROR, "upstream overloaded"),
            ErrorClass::UpstreamRetryable
        );
        // 4xx 非 Key 字样：客户端错误
        assert_eq!(
            classify_upstream(StatusCode::BAD_REQUEST, "model does not exist"),
            ErrorClass::Client
        );
        // 5xx 含余额字样：Key 重试优先
        assert_eq!(
            classify_upstream(StatusCode::INTERNAL_SERVER_ERROR, "insufficient_quota"),
            ErrorClass::KeyRetryable
        );
    }

    #[test]
    fn classify_upstream_chinese_rate_limit_is_key_retryable() {
        // 回归:智谱 1302「您的账户已达到速率限制」流式经 SSE 分支被标成 502,
        // 但 body 含中文限流语义,应识别为 KeyRetryable 触发换 key。
        // 复现自生产 usage_logs(glm-5.2 限流只试 1 个 key 就 502)。
        assert_eq!(
            classify_upstream(
                StatusCode::BAD_GATEWAY,
                "[1302][您的账户已达到速率限制，请您控制请求频率]"
            ),
            ErrorClass::KeyRetryable
        );
        assert_eq!(
            classify_upstream(StatusCode::BAD_GATEWAY, "请求过于频繁，触发速率限制"),
            ErrorClass::KeyRetryable
        );
    }

    #[test]
    fn sse_stream_error_status_classifies_rate_limit_as_429() {
        // SSE 流内错误归因：限流体应归因为 429（触发换 key、不触发 channel 黑名单）
        assert_eq!(
            sse_stream_error_status("[1302][您的账户已达到速率限制]"),
            StatusCode::TOO_MANY_REQUESTS
        );
        // 非限流错误保持 502
        assert_eq!(
            sse_stream_error_status("upstream overloaded"),
            StatusCode::BAD_GATEWAY
        );
    }

    #[test]
    fn render_status_and_message_maps_error_kinds() {
        // NoAvailableChannel → 503 + 原 message
        let (s, m) = render_status_and_message(&ProxyError::NoAvailableChannel("no ch".into()));
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(m, "no ch");

        // UpstreamError → 上游 status + sanitized body
        let (s, m) = render_status_and_message(&ProxyError::UpstreamError {
            status: StatusCode::TOO_MANY_REQUESTS,
            body: r#"{"error":{"message":"rate limit"}}"#.into(),
        });
        assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
        assert!(m.contains("rate limit"));

        // DatabaseError → 500 + 原 message（不是上游问题）
        let (s, m) = render_status_and_message(&ProxyError::DatabaseError("db gone".into()));
        assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(m, "db gone");

        // ChannelNotFound → 404 + 原 message
        let (s, m) = render_status_and_message(&ProxyError::ChannelNotFound("ch-x".into()));
        assert_eq!(s, StatusCode::NOT_FOUND);
        assert_eq!(m, "ch-x");

        // RequestError → 502 + 原 message（网络层错误）
        let (s, m) = render_status_and_message(&ProxyError::RequestError("conn refused".into()));
        assert_eq!(s, StatusCode::BAD_GATEWAY);
        assert_eq!(m, "conn refused");

        // TransformError → 500 + 原 message
        let (s, m) = render_status_and_message(&ProxyError::TransformError("bad json".into()));
        assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(m, "bad json");
    }
}
