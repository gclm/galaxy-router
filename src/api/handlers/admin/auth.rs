use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::error::app::{ApiError, ApiResponse};
use crate::auth::{JwtService, PasswordService};

/// 认证状态
#[derive(Clone)]
pub struct AuthState {
    pub pool: SqlitePool,
    pub jwt_service: JwtService,
}

/// 初始化请求
#[derive(Deserialize)]
pub struct InitRequest {
    pub username: String,
    pub password: String,
    pub site_title: Option<String>,
}

/// 登录请求
#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// 修改密码请求
#[derive(Deserialize)]
pub struct ChangePasswordRequest {
    pub old_password: String,
    pub new_password: String,
}

/// 认证响应
#[derive(Serialize)]
pub struct AuthResponse {
    pub token: String,
    pub expires_in: u64,
}

/// 用户信息响应
#[derive(Serialize)]
pub struct UserInfoResponse {
    pub id: String,
    pub username: String,
}

/// 初始化系统（创建管理员 + 站点配置）
pub async fn init(
    State(state): State<AuthState>,
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

    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    // INSERT ... WHERE NOT EXISTS 防止并发竞态创建多个管理员
    let inserted = sqlx::query(
        "INSERT INTO users (id, username, password_hash) SELECT ?, ?, ? WHERE NOT EXISTS (SELECT 1 FROM users)"
    )
    .bind(&user_id)
    .bind(&req.username)
    .bind(&password_hash)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal_error(e.to_string()))?;

    if inserted.rows_affected() == 0 {
        return Err(ApiError::conflict("系统已初始化，无需重复操作"));
    }

    if let Some(site_title) = &req.site_title {
        sqlx::query(
            "INSERT OR REPLACE INTO settings (key, category, value, description) VALUES ('site.title', 'general', ?, '站点标题')",
        )
        .bind(site_title)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;
    }

    tx.commit()
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

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
    State(state): State<AuthState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<ApiResponse<AuthResponse>>, (StatusCode, Json<ApiError>)> {
    let user = sqlx::query_as::<_, (String, String, String)>(
        "SELECT id, username, password_hash FROM users WHERE username = ?",
    )
    .bind(&req.username)
    .fetch_optional(&state.pool)
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
    State(state): State<AuthState>,
    auth: crate::api::middleware::AuthClaims,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<Json<ApiResponse<()>>, (StatusCode, Json<ApiError>)> {
    let password_hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
        .bind(&auth.0.sub)
        .fetch_one(&state.pool)
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

    sqlx::query("UPDATE users SET password_hash = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(&new_hash)
        .bind(&auth.0.sub)
        .execute(&state.pool)
        .await
        .map_err(|e| ApiError::internal_error(e.to_string()))?;

    Ok(Json(crate::error::app::success_empty()))
}
