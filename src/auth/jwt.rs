use chrono::{Duration, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

/// JWT Claims
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub username: String,
    pub exp: usize,
    pub iat: usize,
}

/// 构建统一的 JWT Validation（HS256 only）
pub fn jwt_validation() -> Validation {
    Validation::new(Algorithm::HS256)
}

/// 解码并验证 JWT token
pub fn decode_jwt(token: &str, secret: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &jwt_validation(),
    )?;
    Ok(token_data.claims)
}

/// JWT 服务
#[derive(Clone)]
pub struct JwtService {
    secret: String,
    expiry_hours: u64,
}

impl JwtService {
    /// 创建 JWT 服务
    pub fn new(secret: &str, expiry_hours: u64) -> Self {
        Self {
            secret: secret.to_string(),
            expiry_hours,
        }
    }

    /// 生成 Token
    pub fn generate_token(
        &self,
        user_id: &str,
        username: &str,
    ) -> Result<String, jsonwebtoken::errors::Error> {
        let now = Utc::now();
        let expires_at = now + Duration::hours(self.expiry_hours as i64);

        let claims = Claims {
            sub: user_id.to_string(),
            username: username.to_string(),
            exp: expires_at.timestamp() as usize,
            iat: now.timestamp() as usize,
        };

        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.secret.as_bytes()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_token_format() {
        let service = JwtService::new("test_secret", 24);
        let token = service.generate_token("1", "admin").unwrap();
        assert!(!token.is_empty());
        assert_eq!(token.split('.').count(), 3);
    }
}
