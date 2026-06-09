use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 粘性会话
#[derive(Debug, Clone)]
pub struct StickySession {
    pub channel_id: String,
    pub expires_at: DateTime<Utc>,
}

/// 粘性会话管理器
#[derive(Clone)]
pub struct StickySessionManager {
    sessions: Arc<RwLock<HashMap<String, StickySession>>>,
    ttl_secs: i64,
    max_sessions: usize,
}

impl Default for StickySessionManager {
    fn default() -> Self {
        Self::new(3600, 10000)
    }
}

impl StickySessionManager {
    pub fn new(ttl_secs: i64, max_sessions: usize) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            ttl_secs,
            max_sessions,
        }
    }

    /// 获取粘性会话
    pub async fn get(&self, session_hash: &str) -> Option<String> {
        let sessions = self.sessions.read().await;
        if let Some(session) = sessions.get(session_hash)
            && Utc::now() < session.expires_at
        {
            return Some(session.channel_id.clone());
        }
        None
    }

    /// 设置粘性会话
    pub async fn set(&self, session_hash: &str, channel_id: &str) {
        let mut sessions = self.sessions.write().await;

        // 容量检查：超过上限时清理过期条目
        if sessions.len() >= self.max_sessions {
            let now = Utc::now();
            sessions.retain(|_, session| now < session.expires_at);

            // 清理后仍然满，拒绝新 session
            if sessions.len() >= self.max_sessions {
                tracing::warn!(
                    "粘性会话已满（{}），拒绝新 session: {}",
                    self.max_sessions,
                    session_hash
                );
                return;
            }
        }

        let now = Utc::now();
        sessions.insert(
            session_hash.to_string(),
            StickySession {
                channel_id: channel_id.to_string(),
                expires_at: now + chrono::Duration::seconds(self.ttl_secs),
            },
        );
    }

    /// 清理过期的粘性会话
    pub async fn cleanup_expired(&self) {
        let mut sessions = self.sessions.write().await;
        let now = Utc::now();
        sessions.retain(|_, session| now < session.expires_at);
    }
}
