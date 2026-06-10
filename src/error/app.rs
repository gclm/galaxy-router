use axum::{Json, http::StatusCode, response::IntoResponse};
use serde::Serialize;

/// 统一成功响应
#[derive(Debug, Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub code: i32,
    pub message: String,
    pub data: Option<T>,
}

impl<T: Serialize> ApiResponse<T> {
    /// 成功响应
    pub fn success(data: T) -> Self {
        Self {
            code: 0,
            message: "success".to_string(),
            data: Some(data),
        }
    }
}

/// 成功响应（无数据）
pub fn success_empty() -> ApiResponse<()> {
    ApiResponse {
        code: 0,
        message: "success".to_string(),
        data: None,
    }
}

/// 统一错误响应
#[derive(Debug, Serialize)]
pub struct ApiError {
    pub code: i32,
    pub message: String,
}

impl ApiError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// 参数错误
    pub fn bad_request(message: impl Into<String>) -> (StatusCode, Json<Self>) {
        (StatusCode::BAD_REQUEST, Json(Self::new(400, message)))
    }

    /// 未授权
    pub fn unauthorized(message: impl Into<String>) -> (StatusCode, Json<Self>) {
        (StatusCode::UNAUTHORIZED, Json(Self::new(401, message)))
    }

    /// 资源不存在
    pub fn not_found(message: impl Into<String>) -> (StatusCode, Json<Self>) {
        (StatusCode::NOT_FOUND, Json(Self::new(404, message)))
    }

    /// 资源冲突
    pub fn conflict(message: impl Into<String>) -> (StatusCode, Json<Self>) {
        (StatusCode::CONFLICT, Json(Self::new(409, message)))
    }

    /// 服务器内部错误
    pub fn internal_error(message: impl Into<String>) -> (StatusCode, Json<Self>) {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(Self::new(500, message)),
        )
    }
}

/// 为 (StatusCode, Json<ApiError>) 实现 IntoResponse
impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(self)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_response_success() {
        let resp = ApiResponse::success("test");
        assert_eq!(resp.code, 0);
        assert_eq!(resp.message, "success");
        assert_eq!(resp.data, Some("test"));
    }

    #[test]
    fn test_api_error() {
        let (status, Json(err)) = ApiError::bad_request("invalid input");
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, 400);
        assert_eq!(err.message, "invalid input");
    }
}
