//! Auth 领域模型（v1.1.2 从 auth.rs 抽出）。

use serde::{Deserialize, Serialize};

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
