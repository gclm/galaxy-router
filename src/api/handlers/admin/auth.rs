//! 鉴权 handler（薄：调 service/auth）。init/login/change_password 编排在 service（D5 归位）。

use axum::{Json, extract::State, http::StatusCode};

use crate::app_state::AppState;
use crate::error::app::{ApiError, ApiResponse};
use crate::service::auth::{AuthError, AuthService};

pub use crate::domain::auth::*;

fn map_auth_err(e: AuthError) -> (StatusCode, Json<ApiError>) {
    match e {
        AuthError::BadRequest(m) => ApiError::bad_request(m),
        AuthError::Conflict(m) => ApiError::conflict(m),
        AuthError::Unauthorized(m) => ApiError::unauthorized(m),
        AuthError::Internal(m) => ApiError::internal_error(m),
    }
}

/// 初始化系统（创建管理员 + 站点配置）
pub async fn init(
    State(state): State<AppState>,
    Json(req): Json<InitRequest>,
) -> Result<(StatusCode, Json<ApiResponse<AuthResponse>>), (StatusCode, Json<ApiError>)> {
    let svc = AuthService::new(state.repositories.auth.clone(), state.jwt_service.clone());
    let resp = svc.init(req).await.map_err(map_auth_err)?;
    Ok((StatusCode::CREATED, Json(ApiResponse::success(resp))))
}

/// 登录
pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<ApiResponse<AuthResponse>>, (StatusCode, Json<ApiError>)> {
    let svc = AuthService::new(state.repositories.auth.clone(), state.jwt_service.clone());
    let resp = svc.login(req).await.map_err(map_auth_err)?;
    Ok(Json(ApiResponse::success(resp)))
}

/// 获取当前用户信息
pub async fn me(
    auth: crate::api::middleware::AuthClaims,
) -> Result<Json<ApiResponse<UserInfoResponse>>, (StatusCode, Json<ApiError>)> {
    Ok(Json(ApiResponse::success(UserInfoResponse {
        id: auth.0.sub,
        username: auth.0.username,
    })))
}

/// 修改密码
pub async fn change_password(
    State(state): State<AppState>,
    auth: crate::api::middleware::AuthClaims,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<Json<ApiResponse<()>>, (StatusCode, Json<ApiError>)> {
    let svc = AuthService::new(state.repositories.auth.clone(), state.jwt_service.clone());
    svc.change_password(&auth.0.sub, req)
        .await
        .map_err(map_auth_err)?;
    Ok(Json(crate::error::app::success_empty()))
}
