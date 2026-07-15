//! 鉴权 service：init/login/change_password 编排（D5 归位）。
//!
//! 复用 PasswordService（hash/verify）+ JwtService（generate_token）+ AuthRepository。
//! handler 只做 HTTP 适配（map AuthError → ApiError）；me 留 handler（读 AuthClaims）。

use std::sync::Arc;

use crate::auth::{JwtService, PasswordService};
use crate::domain::auth::{AuthResponse, ChangePasswordRequest, InitRequest, LoginRequest};
use crate::repository::auth_repository::AuthRepository;

#[derive(Clone)]
pub struct AuthService {
    repo: Arc<dyn AuthRepository>,
    jwt: JwtService,
}

impl AuthService {
    pub fn new(repo: Arc<dyn AuthRepository>, jwt: JwtService) -> Self {
        Self { repo, jwt }
    }

    /// 初始化系统：校验 → hash → 写首个管理员（幂等）→ 签发 token。
    pub async fn init(&self, req: InitRequest) -> Result<AuthResponse, AuthError> {
        if req.username.len() < 3 {
            return Err(AuthError::BadRequest("用户名至少 3 个字符".into()));
        }
        if req.password.len() < 8 {
            return Err(AuthError::BadRequest("密码至少 8 个字符".into()));
        }

        let password_hash = PasswordService::hash_password(&req.password)
            .map_err(|e| AuthError::Internal(e.to_string()))?;
        let user_id = crate::util::id::generate_id();

        let inserted = self
            .repo
            .init(&user_id, &req.username, &password_hash, req.site_title.as_deref())
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;
        if !inserted {
            return Err(AuthError::Conflict("系统已初始化，无需重复操作".into()));
        }

        let token = self
            .jwt
            .generate_token(&user_id, &req.username)
            .map_err(|e| AuthError::Internal(e.to_string()))?;
        Ok(AuthResponse {
            token,
            expires_in: 86400,
        })
    }

    /// 登录：查用户 → 校验密码 → 签发 token。
    pub async fn login(&self, req: LoginRequest) -> Result<AuthResponse, AuthError> {
        let user = self
            .repo
            .find_by_username(&req.username)
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;
        let (user_id, username, password_hash) = user
            .ok_or_else(|| AuthError::Unauthorized("用户名或密码错误".into()))?;

        let valid = PasswordService::verify_password(&req.password, &password_hash)
            .map_err(|e| AuthError::Internal(e.to_string()))?;
        if !valid {
            return Err(AuthError::Unauthorized("用户名或密码错误".into()));
        }

        let token = self
            .jwt
            .generate_token(&user_id, &username)
            .map_err(|e| AuthError::Internal(e.to_string()))?;
        Ok(AuthResponse {
            token,
            expires_in: 86400,
        })
    }

    /// 修改密码：校验旧密码 → 校验新密码长度 → hash → 更新。
    pub async fn change_password(
        &self,
        user_id: &str,
        req: ChangePasswordRequest,
    ) -> Result<(), AuthError> {
        let password_hash = self
            .repo
            .get_password_hash(user_id)
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;
        let valid = PasswordService::verify_password(&req.old_password, &password_hash)
            .map_err(|e| AuthError::Internal(e.to_string()))?;
        if !valid {
            return Err(AuthError::Unauthorized("旧密码错误".into()));
        }
        if req.new_password.len() < 8 {
            return Err(AuthError::BadRequest("新密码至少 8 个字符".into()));
        }

        let new_hash = PasswordService::hash_password(&req.new_password)
            .map_err(|e| AuthError::Internal(e.to_string()))?;
        self.repo
            .update_password(user_id, &new_hash)
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;
        Ok(())
    }
}

pub enum AuthError {
    BadRequest(String),
    Conflict(String),
    Unauthorized(String),
    Internal(String),
}
