use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 渠道状态
#[derive(Debug, Clone)]
pub struct ChannelStatus {
    pub channel_id: String,
    pub success_count: u64,
    pub failure_count: u64,
    pub last_success: Option<DateTime<Utc>>,
    pub last_failure: Option<DateTime<Utc>>,
    pub avg_latency_ms: f64,
    pub is_blacklisted: bool,
    pub blacklist_until: Option<DateTime<Utc>>,
}

impl ChannelStatus {
    #[allow(dead_code)]
    pub fn new(channel_id: String) -> Self {
        Self {
            channel_id,
            success_count: 0,
            failure_count: 0,
            last_success: None,
            last_failure: None,
            avg_latency_ms: 0.0,
            is_blacklisted: false,
            blacklist_until: None,
        }
    }

    /// 计算错误率
    pub fn error_rate(&self) -> f64 {
        let total = self.success_count + self.failure_count;
        if total == 0 {
            return 0.0;
        }
        self.failure_count as f64 / total as f64
    }

    /// 记录成功
    pub fn record_success(&mut self, latency_ms: f64) {
        self.success_count += 1;
        self.last_success = Some(Utc::now());

        // 成功时将失败计数减半（衰减而非清零），避免单次成功洗白
        self.failure_count = self.failure_count.saturating_div(2);

        // 解除拉黑
        if self.failure_count == 0 {
            self.is_blacklisted = false;
            self.blacklist_until = None;
        }

        // 更新平均延迟（指数移动平均）
        if self.avg_latency_ms == 0.0 {
            self.avg_latency_ms = latency_ms;
        } else {
            self.avg_latency_ms = 0.8 * self.avg_latency_ms + 0.2 * latency_ms;
        }
    }

    /// 记录失败
    pub fn record_failure(&mut self) {
        self.failure_count += 1;
        self.last_failure = Some(Utc::now());
    }

    /// 拉黑
    pub fn blacklist(&mut self, minutes: i64) {
        self.is_blacklisted = true;
        self.blacklist_until = Some(Utc::now() + chrono::Duration::minutes(minutes));
    }

    /// 检查是否可用
    pub fn is_available(&self) -> bool {
        if !self.is_blacklisted {
            return true;
        }

        // 检查拉黑是否过期
        if let Some(until) = self.blacklist_until {
            Utc::now() >= until
        } else {
            false
        }
    }
}

/// 粘性会话
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct StickySession {
    pub session_hash: String,
    pub channel_id: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// 负载均衡状态
#[derive(Clone)]
pub struct LoadBalancerState {
    /// 渠道状态
    pub channel_states: Arc<RwLock<HashMap<String, ChannelStatus>>>,
    /// 粘性会话
    pub sticky_sessions: Arc<RwLock<HashMap<String, StickySession>>>,
    /// 粘性会话 TTL（秒）
    pub sticky_ttl_secs: i64,
    /// 拉黑阈值（连续失败次数）
    pub blacklist_threshold: u64,
    /// 拉黑时长（分钟）
    pub blacklist_minutes: i64,
}

impl Default for LoadBalancerState {
    fn default() -> Self {
        Self::new()
    }
}

impl LoadBalancerState {
    pub fn new() -> Self {
        Self {
            channel_states: Arc::new(RwLock::new(HashMap::new())),
            sticky_sessions: Arc::new(RwLock::new(HashMap::new())),
            sticky_ttl_secs: 3600,
            blacklist_threshold: 3,
            blacklist_minutes: 10,
        }
    }

    /// 获取或创建渠道状态
    #[allow(dead_code)]
    pub async fn get_or_create_channel_status(&self, channel_id: &str) -> ChannelStatus {
        let mut states = self.channel_states.write().await;
        states
            .entry(channel_id.to_string())
            .or_insert_with(|| ChannelStatus::new(channel_id.to_string()))
            .clone()
    }

    /// 记录请求成功
    pub async fn record_success(&self, channel_id: &str, latency_ms: f64) {
        let mut states = self.channel_states.write().await;
        if let Some(status) = states.get_mut(channel_id) {
            status.record_success(latency_ms);
        }
    }

    /// 记录请求失败
    pub async fn record_failure(&self, channel_id: &str, should_blacklist: bool) {
        let mut states = self.channel_states.write().await;
        if let Some(status) = states.get_mut(channel_id) {
            status.record_failure();

            // 检查是否需要拉黑
            if should_blacklist && status.failure_count >= self.blacklist_threshold {
                status.blacklist(self.blacklist_minutes);
                tracing::warn!("渠道 {} 被拉黑 {} 分钟", channel_id, self.blacklist_minutes);
            }
        }
    }

    /// 计算渠道评分
    pub async fn calculate_score(&self, channel_id: &str, base_weight: i32) -> f64 {
        let states = self.channel_states.read().await;
        let status = states.get(channel_id);

        let mut score = base_weight as f64;

        if let Some(status) = status {
            // 不可用渠道评分归零
            if !status.is_available() {
                return 0.0;
            }

            // 错误率惩罚
            let error_rate = status.error_rate();
            score *= 1.0 - error_rate;

            // 延迟惩罚（延迟越高，评分越低）
            if status.avg_latency_ms > 0.0 {
                let latency_factor = 1.0 / (1.0 + status.avg_latency_ms / 1000.0);
                score *= latency_factor;
            }
        }

        score.max(0.0)
    }

    /// 获取粘性会话
    pub async fn get_sticky_session(&self, session_hash: &str) -> Option<String> {
        let sessions = self.sticky_sessions.read().await;
        if let Some(session) = sessions.get(session_hash)
            && Utc::now() < session.expires_at
        {
            return Some(session.channel_id.clone());
        }
        None
    }

    /// 设置粘性会话
    pub async fn set_sticky_session(&self, session_hash: &str, channel_id: &str) {
        let mut sessions = self.sticky_sessions.write().await;
        let now = Utc::now();
        sessions.insert(
            session_hash.to_string(),
            StickySession {
                session_hash: session_hash.to_string(),
                channel_id: channel_id.to_string(),
                created_at: now,
                expires_at: now + chrono::Duration::seconds(self.sticky_ttl_secs),
            },
        );
    }

    /// 清理过期的粘性会话
    pub async fn cleanup_expired_sessions(&self) {
        let mut sessions = self.sticky_sessions.write().await;
        let now = Utc::now();
        sessions.retain(|_, session| now < session.expires_at);
    }

    /// 清理过期的拉黑
    pub async fn cleanup_expired_blacklists(&self) {
        let mut states = self.channel_states.write().await;
        for (_, status) in states.iter_mut() {
            if status.is_blacklisted
                && let Some(until) = status.blacklist_until
                && Utc::now() >= until
            {
                status.is_blacklisted = false;
                status.blacklist_until = None;
                status.failure_count = 0; // 重置失败计数
                tracing::info!("渠道 {} 拉黑已过期，恢复正常", status.channel_id);
            }
        }
    }
}
