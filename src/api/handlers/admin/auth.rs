use axum::{Json, extract::State, http::StatusCode};

use crate::app_state::AppState;
use crate::auth::PasswordService;
use crate::error::app::{ApiError, ApiResponse};

pub use crate::domain::auth::*;

/// 初始化系统（创建管理员 + 站点配置）
pub async fn init(
    State(state): State<AppState>,
    Json(req): Json<InitRequest>,
) -> Result<(StatusCode, Json<ApiResponse<AuthResponse>>), (StatusCode, Json<ApiError>)> {
    if req.username.len() < 3 {
        return Err(ApiError::bad_request("用户名至少 3 个字符"));
    }
    if req.password.len() < 8 {
        return Err(ApiError::bad_request("密码至少 8 个字符"));
    }

    let password_hash = PasswordService::hash_password(&req.password)
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    let user_id = crate::api::response::generate_id();

    let inserted = state
        .repositories
        .auth
        .init(&user_id, &req.username, &password_hash, req.site_title.as_deref())
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    if !inserted {
        return Err(ApiError::conflict("系统已初始化，无需重复操作"));
    }

    let token = state
        .jwt_service
        .generate_token(&user_id, &req.username)
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(ApiResponse::success(AuthResponse {
            token,
            expires_in: 86400,
        })),
    ))
}

/// 登录
pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<ApiResponse<AuthResponse>>, (StatusCode, Json<ApiError>)> {
    let user = state
        .repositories
        .auth
        .find_by_username(&req.username)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    let (user_id, username, password_hash) =
        user.ok_or_else(|| ApiError::unauthorized("用户名或密码错误"))?;

    let valid = PasswordService::verify_password(&req.password, &password_hash)
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    if !valid {
        return Err(ApiError::unauthorized("用户名或密码错误"));
    }

    let token = state
        .jwt_service
        .generate_token(&user_id, &username)
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::success(AuthResponse {
        token,
        expires_in: 86400,
    })))
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
    let password_hash = state
        .repositories
        .auth
        .get_password_hash(&auth.0.sub)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    let valid = PasswordService::verify_password(&req.old_password, &password_hash)
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    if !valid {
        return Err(ApiError::unauthorized("旧密码错误"));
    }

    if req.new_password.len() < 8 {
        return Err(ApiError::bad_request("新密码至少 8 个字符"));
    }

    let new_hash = PasswordService::hash_password(&req.new_password)
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    state
        .repositories
        .auth
        .update_password(&auth.0.sub, &new_hash)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    Ok(Json(crate::error::app::success_empty()))
}
