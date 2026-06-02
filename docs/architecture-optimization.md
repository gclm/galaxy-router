# Galaxy Proxy 架构优化方案

> 基于 Octopus、New API、Sub2API 三个参考项目的最佳实践，针对当前项目的 3 项核心改进。

## 目录

1. [三态熔断器 + 指数退避](#1-三态熔断器--指数退避)
2. [JSON 日志脱敏](#2-json-日志脱敏)
3. [Token 估算精度提升](#3-token-估算精度提升)
4. [实施计划](#4-实施计划)

---

## 1. 三态熔断器 + 指数退避

### 1.1 当前问题

当前 `LoadBalancerState` 使用简单的二态模型（可用/拉黑）：

```
可用 ──(连续失败≥3)──→ 拉黑 ──(10分钟后)──→ 可用
```

**问题**：
- 拉黑恢复后立即承受全量流量，如果上游未完全恢复会再次快速拉黑
- 拉黑/恢复循环频繁，影响服务质量
- 固定 10 分钟拉黑时长，无法适应不同故障场景

### 1.2 参考方案：Octopus 三态熔断器

Octopus 实现了经典的三态熔断器模式：

```
                    ┌─────────────────┐
                    │                 │
                    ▼                 │
              ┌──────────┐     ┌──────────┐     ┌──────────┐
              │  Closed  │────▶│   Open   │────▶│ HalfOpen │
              │  (正常)  │     │  (熔断)  │     │  (试探)  │
              └──────────┘     └──────────┘     └──────────┘
                    ▲                 ▲                │
                    │                 │                │
                    └─────────────────┴────────────────┘
                              试探失败
```

**关键特性**：
- **Closed**：正常状态，统计连续失败次数
- **Open**：熔断状态，拒绝所有请求，等待冷却时间
- **HalfOpen**：半开状态，允许单个试探请求

**指数退避**：
```go
cooldown = baseCooldown * 2^(tripCount - 1)
// 示例：base=60s, max=600s
// 第1次熔断：60s
// 第2次熔断：120s
// 第3次熔断：240s
// 第4次熔断：480s
// 第5次及以后：600s（上限）
```

### 1.3 Galaxy Proxy 改造方案

#### 1.3.1 数据结构

```rust
// src/proxy/circuit.rs

/// 熔断器状态
#[derive(Debug, Clone, PartialEq)]
pub enum CircuitState {
    /// 正常通行
    Closed,
    /// 熔断中，拒绝所有请求
    Open,
    /// 半开，仅允许单个试探请求
    HalfOpen,
}

/// 熔断器条目
#[derive(Debug, Clone)]
pub struct CircuitEntry {
    pub state: CircuitState,
    pub consecutive_failures: u64,
    pub last_failure_time: Option<Instant>,
    pub trip_count: u32,  // 累计熔断次数（用于指数退避）
    pub half_open_probe: bool,  // 是否有试探请求进行中
}

/// 熔断器配置
pub struct CircuitConfig {
    /// 触发熔断的连续失败次数
    pub failure_threshold: u64,
    /// 基础冷却时间（秒）
    pub base_cooldown_secs: u64,
    /// 最大冷却时间（秒）
    pub max_cooldown_secs: u64,
    /// HalfOpen 状态试探超时（秒）
    pub probe_timeout_secs: u64,
}

impl Default for CircuitConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            base_cooldown_secs: 60,
            max_cooldown_secs: 600,
            probe_timeout_secs: 30,
        }
    }
}
```

#### 1.3.2 熔断器实现

```rust
// src/proxy/circuit.rs

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::time::{Duration, Instant};

/// 熔断器键：channel_id:key_hint
type CircuitKey = String;

/// 全局熔断器存储
#[derive(Clone)]
pub struct CircuitBreaker {
    entries: Arc<RwLock<HashMap<CircuitKey, CircuitEntry>>>,
    config: CircuitConfig,
}

impl CircuitBreaker {
    pub fn new(config: CircuitConfig) -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            config,
        }
    }

    /// 计算冷却时间（指数退避）
    fn cooldown_duration(&self, trip_count: u32) -> Duration {
        if trip_count == 0 {
            return Duration::from_secs(self.config.base_cooldown_secs);
        }
        
        // base * 2^(trip_count - 1)，防止溢出
        let shift = (trip_count - 1).min(20);
        let cooldown = self.config.base_cooldown_secs * (1 << shift);
        let cooldown = cooldown.min(self.config.max_cooldown_secs);
        
        Duration::from_secs(cooldown)
    }

    /// 检查渠道是否被熔断
    /// 返回 (is_tripped, remaining_cooldown)
    pub async fn is_tripped(&self, channel_id: &str, key_hint: &str) -> (bool, Option<Duration>) {
        let key = format!("{}:{}", channel_id, key_hint);
        let entries = self.entries.read().await;
        
        let Some(entry) = entries.get(&key) else {
            return (false, None);
        };

        match entry.state {
            CircuitState::Closed => (false, None),
            CircuitState::Open => {
                let cooldown = self.cooldown_duration(entry.trip_count);
                let elapsed = entry.last_failure_time
                    .map(|t| t.elapsed())
                    .unwrap_or(Duration::from_secs(u64::MAX));
                
                if elapsed >= cooldown {
                    // 冷却时间已过，转为 HalfOpen
                    drop(entries);
                    let mut entries = self.entries.write().await;
                    if let Some(entry) = entries.get_mut(&key) {
                        entry.state = CircuitState::HalfOpen;
                        entry.half_open_probe = false;
                        tracing::info!(
                            "circuit [{}] Open -> HalfOpen (cooldown {:?} elapsed)",
                            key, cooldown
                        );
                    }
                    (false, None)
                } else {
                    (true, Some(cooldown - elapsed))
                }
            }
            CircuitState::HalfOpen => {
                if entry.half_open_probe {
                    // 已有试探请求在进行中，拒绝其他请求
                    (true, None)
                } else {
                    (false, None)
                }
            }
        }
    }

    /// 开始试探请求（HalfOpen -> 标记试探中）
    pub async fn begin_probe(&self, channel_id: &str, key_hint: &str) -> bool {
        let key = format!("{}:{}", channel_id, key_hint);
        let mut entries = self.entries.write().await;
        
        if let Some(entry) = entries.get_mut(&key) {
            if entry.state == CircuitState::HalfOpen && !entry.half_open_probe {
                entry.half_open_probe = true;
                return true;
            }
        }
        false
    }

    /// 记录成功
    pub async fn record_success(&self, channel_id: &str, key_hint: &str) {
        let key = format!("{}:{}", channel_id, key_hint);
        let mut entries = self.entries.write().await;
        
        if let Some(entry) = entries.get_mut(&key) {
            if entry.state == CircuitState::HalfOpen {
                tracing::info!("circuit [{}] HalfOpen -> Closed (probe succeeded)", key);
            }
            // 重置全部状态
            entry.state = CircuitState::Closed;
            entry.consecutive_failures = 0;
            entry.trip_count = 0;
            entry.half_open_probe = false;
        }
    }

    /// 记录失败
    pub async fn record_failure(&self, channel_id: &str, key_hint: &str) {
        let key = format!("{}:{}", channel_id, key_hint);
        let mut entries = self.entries.write().await;
        
        let entry = entries.entry(key.clone()).or_insert_with(|| CircuitEntry {
            state: CircuitState::Closed,
            consecutive_failures: 0,
            last_failure_time: None,
            trip_count: 0,
            half_open_probe: false,
        });

        entry.last_failure_time = Some(Instant::now());

        match entry.state {
            CircuitState::Closed => {
                entry.consecutive_failures += 1;
                if entry.consecutive_failures >= self.config.failure_threshold {
                    entry.state = CircuitState::Open;
                    entry.trip_count += 1;
                    let cooldown = self.cooldown_duration(entry.trip_count);
                    tracing::warn!(
                        "circuit [{}] Closed -> Open (failures={}, trip_count={}, cooldown={:?})",
                        key, entry.consecutive_failures, entry.trip_count, cooldown
                    );
                }
            }
            CircuitState::HalfOpen => {
                // 试探失败，重新进入 Open，trip_count 递增
                entry.state = CircuitState::Open;
                entry.trip_count += 1;
                entry.consecutive_failures = 0;
                entry.half_open_probe = false;
                let cooldown = self.cooldown_duration(entry.trip_count);
                tracing::warn!(
                    "circuit [{}] HalfOpen -> Open (probe failed, trip_count={}, cooldown={:?})",
                    key, entry.trip_count, cooldown
                );
            }
            CircuitState::Open => {
                // 理论上不应在 Open 状态收到失败记录
            }
        }
    }

    /// 清理过期条目（可选，定期调用）
    pub async fn cleanup_expired(&self, max_age: Duration) {
        let mut entries = self.entries.write().await;
        let now = Instant::now();
        entries.retain(|_, entry| {
            entry.last_failure_time
                .map(|t| now.duration_since(t) < max_age)
                .unwrap_or(true)
        });
    }
}
```

#### 1.3.3 与现有代码集成

修改 `src/proxy/state.rs`：

```rust
// 替换现有的 LoadBalancerState 中的拉黑逻辑

/// 负载均衡状态
#[derive(Clone)]
pub struct LoadBalancerState {
    /// 渠道状态（保留用于统计）
    pub channel_states: Arc<RwLock<HashMap<String, ChannelStatus>>>,
    /// 粘性会话
    pub sticky_sessions: Arc<RwLock<HashMap<String, StickySession>>>,
    /// 熔断器（新增）
    pub circuit_breaker: CircuitBreaker,
    /// 配置
    pub config: LoadBalancerConfig,
}

impl LoadBalancerState {
    /// 检查渠道是否可用（使用熔断器）
    pub async fn is_channel_available(&self, channel_id: &str, key_hint: &str) -> bool {
        let (tripped, _) = self.circuit_breaker.is_tripped(channel_id, key_hint).await;
        !tripped
    }

    /// 记录请求成功
    pub async fn record_success(&self, channel_id: &str, key_hint: &str, latency_ms: f64) {
        // 更新统计
        if let Some(status) = self.channel_states.write().await.get_mut(channel_id) {
            status.record_success(latency_ms);
        }
        // 通知熔断器
        self.circuit_breaker.record_success(channel_id, key_hint).await;
    }

    /// 记录请求失败
    pub async fn record_failure(&self, channel_id: &str, key_hint: &str) {
        // 更新统计
        if let Some(status) = self.channel_states.write().await.get_mut(channel_id) {
            status.record_failure();
        }
        // 通知熔断器
        self.circuit_breaker.record_failure(channel_id, key_hint).await;
    }
}
```

#### 1.3.4 配置项

在 `config.toml` 中添加：

```toml
[load_balancer]
# 熔断器配置
circuit_failure_threshold = 5      # 触发熔断的连续失败次数
circuit_base_cooldown_secs = 60    # 基础冷却时间（秒）
circuit_max_cooldown_secs = 600    # 最大冷却时间（秒）
circuit_probe_timeout_secs = 30    # 试探超时（秒）

# 粘性会话配置
sticky_ttl_secs = 3600
max_sticky_sessions = 10000
```

---

## 2. JSON 日志脱敏

### 2.1 当前问题

当前 `request_content` 和 `response_content` 完整存储到数据库：

```rust
// src/proxy/execute.rs:158
let request_content_clone = serde_json::to_string(&body).ok();
```

**风险**：
- 请求中可能包含 API Key、Authorization token、密码等敏感信息
- 日志查询 API 返回时可能泄露
- 数据库备份/导出时可能泄露

### 2.2 参考方案：Sub2API 日志脱敏

Sub2API 实现了递归 JSON 脱敏：

```go
// 敏感字段列表（key 匹配，不区分大小写）
var sensitiveKeys = []string{
    "authorization", "access_token", "refresh_token", 
    "id_token", "session_token", "token", "client_secret", 
    "private_key", "signature",
}

// 保护字段（不脱敏，即使包含 "token"）
var protectedKeys = []string{
    "max_tokens", "prompt_tokens", "completion_tokens", 
    "input_tokens", "output_tokens", "budget_tokens",
}
```

### 2.3 Galaxy Proxy 改造方案

#### 2.3.1 脱敏模块

```rust
// src/stats/redaction.rs

use serde_json::Value;

/// 敏感字段关键词（小写匹配）
const SENSITIVE_KEYWORDS: &[&str] = &[
    "authorization",
    "access_token",
    "refresh_token",
    "id_token",
    "session_token",
    "client_secret",
    "private_key",
    "signature",
    "api_key",
    "apikey",
    "secret",
    "password",
    "passwd",
    "credential",
];

/// 保护字段（不脱敏，即使包含敏感关键词）
const PROTECTED_KEYS: &[&str] = &[
    "max_tokens",
    "max_output_tokens",
    "max_input_tokens",
    "max_completion_tokens",
    "max_tokens_to_sample",
    "budget_tokens",
    "prompt_tokens",
    "completion_tokens",
    "input_tokens",
    "output_tokens",
    "total_tokens",
    "token_count",
    "cache_read_tokens",
    "cache_creation_tokens",
];

/// 日志内容最大长度（字节）
const MAX_CONTENT_LENGTH: usize = 16 * 1024;  // 16KB

/// 脱敏标记
const REDACTED: &str = "[REDACTED]";

/// 检查 key 是否为敏感字段
fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_lowercase();
    
    // 先检查是否为保护字段
    if PROTECTED_KEYS.iter().any(|p| lower == *p) {
        return false;
    }
    
    // 检查是否包含敏感关键词
    SENSITIVE_KEYWORDS.iter().any(|s| lower.contains(s))
}

/// 递归脱敏 JSON 值
fn redact_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, val) in map.iter_mut() {
                if is_sensitive_key(key) {
                    *val = Value::String(REDACTED.to_string());
                } else {
                    redact_value(val);
                }
            }
        }
        Value::Array(arr) => {
            for item in arr.iter_mut() {
                redact_value(item);
            }
        }
        _ => {}
    }
}

/// 脱敏并截断 JSON 内容
pub fn sanitize_json_content(content: &str) -> String {
    if content.is_empty() {
        return String::new();
    }
    
    // 尝试解析 JSON
    match serde_json::from_str::<Value>(content) {
        Ok(mut value) => {
            redact_value(&mut value);
            let sanitized = serde_json::to_string(&value).unwrap_or_default();
            
            // 截断到最大长度
            if sanitized.len() > MAX_CONTENT_LENGTH {
                format!("{}...", &sanitized[..MAX_CONTENT_LENGTH])
            } else {
                sanitized
            }
        }
        Err(_) => {
            // 非 JSON 内容，直接截断
            if content.len() > MAX_CONTENT_LENGTH {
                format!("{}...", &content[..MAX_CONTENT_LENGTH])
            } else {
                content.to_string()
            }
        }
    }
}

/// 提取关键信息（用于日志摘要）
pub fn extract_summary(content: &str) -> Option<String> {
    let value: Value = serde_json::from_str(content).ok()?;
    
    let mut summary = serde_json::Map::new();
    
    // 提取 model
    if let Some(model) = value.get("model").and_then(|v| v.as_str()) {
        summary.insert("model".to_string(), Value::String(model.to_string()));
    }
    
    // 提取 messages 数量
    if let Some(messages) = value.get("messages").and_then(|v| v.as_array()) {
        summary.insert(
            "messages_count".to_string(),
            Value::Number(messages.len().into()),
        );
    }
    
    // 提取 stream 标志
    if let Some(stream) = value.get("stream").and_then(|v| v.as_bool()) {
        summary.insert("stream".to_string(), Value::Bool(stream));
    }
    
    // 提取 max_tokens
    if let Some(max_tokens) = value.get("max_tokens").and_then(|v| v.as_i64()) {
        summary.insert(
            "max_tokens".to_string(),
            Value::Number(max_tokens.into()),
        );
    }
    
    Some(serde_json::to_string(&summary).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_redact_sensitive_fields() {
        let mut value = json!({
            "model": "gpt-4",
            "authorization": "Bearer sk-xxx",
            "api_key": "sk-yyy",
            "messages": [
                {
                    "role": "user",
                    "content": "hello"
                }
            ],
            "max_tokens": 100
        });
        
        redact_value(&mut value);
        
        assert_eq!(value["model"], "gpt-4");
        assert_eq!(value["authorization"], REDACTED);
        assert_eq!(value["api_key"], REDACTED);
        assert_eq!(value["messages"][0]["content"], "hello");
        assert_eq!(value["max_tokens"], 100);  // 保护字段不脱敏
    }

    #[test]
    fn test_protected_token_fields() {
        let mut value = json!({
            "prompt_tokens": 10,
            "completion_tokens": 20,
            "total_tokens": 30
        });
        
        redact_value(&mut value);
        
        assert_eq!(value["prompt_tokens"], 10);
        assert_eq!(value["completion_tokens"], 20);
        assert_eq!(value["total_tokens"], 30);
    }

    #[test]
    fn test_sanitize_json_content_truncates() {
        let long_content = format!("{{\"data\":\"{}\"}}", "x".repeat(20000));
        let result = sanitize_json_content(&long_content);
        assert!(result.len() <= MAX_CONTENT_LENGTH + 10); // +10 for "..."
    }

    #[test]
    fn test_extract_summary() {
        let content = r#"{"model":"gpt-4","messages":[{"role":"user","content":"hi"}],"stream":true,"max_tokens":100}"#;
        let summary = extract_summary(content).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&summary).unwrap();
        
        assert_eq!(parsed["model"], "gpt-4");
        assert_eq!(parsed["messages_count"], 1);
        assert_eq!(parsed["stream"], true);
        assert_eq!(parsed["max_tokens"], 100);
    }
}
```

#### 2.3.2 集成到 execute.rs

修改 `src/proxy/execute.rs`：

```rust
use crate::stats::redaction::sanitize_json_content;

// 在 save_request_record 中使用脱敏
fn from_last_attempt(
    // ...
    request_content: Option<String>,
    response_content: Option<String>,
    // ...
) -> Self {
    Self {
        // ...
        request_content: request_content.map(|c| sanitize_json_content(&c)),
        response_content: response_content.map(|c| sanitize_json_content(&c)),
        // ...
    }
}
```

#### 2.3.3 配置项

在 `config.toml` 中添加：

```toml
[logging]
# 日志脱敏配置
redact_sensitive_fields = true      # 是否启用脱敏
max_content_length = 16384          # 最大内容长度（字节）
```

---

## 3. Token 估算精度提升

### 3.1 当前问题

当前使用简单的 3 字节/token 估算：

```rust
// src/proxy/prepare.rs:50
pub(super) fn estimate_tokens(text: &str) -> i32 {
    if text.is_empty() { return 0; }
    ((text.len() as f64) / 3.0).ceil() as i32
}
```

**问题**：
- 纯英文：实际约 4 字节/token，估算偏低
- 纯中文：实际约 1.5-2 字节/token，估算偏高
- 混合内容：误差不可控

### 3.2 参考方案：New API 多维度加权

New API 实现了精细的字符类型分类：

| 字符类型 | OpenAI 权重 | Claude 权重 | Gemini 权重 |
|----------|-------------|-------------|-------------|
| 英文单词 | 1.02 | 1.13 | 1.15 |
| 数字 | 1.55 | 1.63 | 2.8 |
| CJK 字符 | 0.85 | 1.21 | 0.68 |
| 普通标点 | 0.4 | 0.4 | 0.38 |
| 数学符号 | 2.68 | 4.52 | 1.05 |
| Emoji | 2.12 | 2.6 | 1.08 |

### 3.3 Galaxy Proxy 改造方案

#### 3.3.1 Token 估算器

```rust
// src/stats/token_estimator.rs

use std::collections::HashMap;

/// 模型厂商
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Provider {
    OpenAI,
    Claude,
    Gemini,
    Unknown,
}

/// 厂商权重配置
#[derive(Debug, Clone)]
pub struct TokenWeights {
    /// 英文单词（每个单词）
    pub word: f64,
    /// 数字（每个连续数字串）
    pub number: f64,
    /// CJK 字符（每个字符）
    pub cjk: f64,
    /// 普通标点符号
    pub symbol: f64,
    /// 数学符号
    pub math_symbol: f64,
    /// URL 分隔符
    pub url_delim: f64,
    /// @ 符号
    pub at_sign: f64,
    /// Emoji
    pub emoji: f64,
    /// 换行符/制表符
    pub newline: f64,
    /// 空格
    pub space: f64,
    /// 基础 padding
    pub base_pad: i32,
}

impl Default for TokenWeights {
    fn default() -> Self {
        Self::openai()
    }
}

impl TokenWeights {
    pub fn openai() -> Self {
        Self {
            word: 1.02,
            number: 1.55,
            cjk: 0.85,
            symbol: 0.4,
            math_symbol: 2.68,
            url_delim: 1.0,
            at_sign: 2.0,
            emoji: 2.12,
            newline: 0.5,
            space: 0.42,
            base_pad: 0,
        }
    }

    pub fn claude() -> Self {
        Self {
            word: 1.13,
            number: 1.63,
            cjk: 1.21,
            symbol: 0.4,
            math_symbol: 4.52,
            url_delim: 1.26,
            at_sign: 2.82,
            emoji: 2.6,
            newline: 0.89,
            space: 0.39,
            base_pad: 0,
        }
    }

    pub fn gemini() -> Self {
        Self {
            word: 1.15,
            number: 2.8,
            cjk: 0.68,
            symbol: 0.38,
            math_symbol: 1.05,
            url_delim: 1.2,
            at_sign: 2.5,
            emoji: 1.08,
            newline: 1.15,
            space: 0.2,
            base_pad: 0,
        }
    }
}

/// Token 估算器
pub struct TokenEstimator {
    weights: HashMap<Provider, TokenWeights>,
}

impl TokenEstimator {
    pub fn new() -> Self {
        let mut weights = HashMap::new();
        weights.insert(Provider::OpenAI, TokenWeights::openai());
        weights.insert(Provider::Claude, TokenWeights::claude());
        weights.insert(Provider::Gemini, TokenWeights::gemini());
        Self { weights }
    }

    /// 根据模型名称推断厂商
    pub fn detect_provider(model: &str) -> Provider {
        let lower = model.to_lowercase();
        if lower.contains("gpt") || lower.contains("o1") || lower.contains("o3") || lower.contains("o4") {
            Provider::OpenAI
        } else if lower.contains("claude") {
            Provider::Claude
        } else if lower.contains("gemini") {
            Provider::Gemini
        } else {
            Provider::Unknown
        }
    }

    /// 获取权重配置
    fn get_weights(&self, provider: &Provider) -> &TokenWeights {
        self.weights.get(provider).unwrap_or(self.weights.get(&Provider::OpenAI).unwrap())
    }

    /// 估算 token 数量
    pub fn estimate(&self, text: &str, provider: &Provider) -> i32 {
        if text.is_empty() {
            return 0;
        }

        let w = self.get_weights(provider);
        let mut count: f64 = 0.0;

        // 状态机：当前单词类型
        enum WordType {
            None,
            Latin,
            Number,
        }
        let mut current_type = WordType::None;

        for ch in text.chars() {
            // 1. 空格和换行符
            if ch.is_whitespace() {
                current_type = WordType::None;
                if ch == '\n' || ch == '\t' {
                    count += w.newline;
                } else {
                    count += w.space;
                }
                continue;
            }

            // 2. CJK 字符
            if is_cjk(ch) {
                current_type = WordType::None;
                count += w.cjk;
                continue;
            }

            // 3. Emoji
            if is_emoji(ch) {
                current_type = WordType::None;
                count += w.emoji;
                continue;
            }

            // 4. 拉丁字母和数字
            if ch.is_alphanumeric() {
                let is_num = ch.is_numeric();
                let new_type = if is_num { WordType::Number } else { WordType::Latin };

                // 单词边界检测
                match current_type {
                    WordType::None => {
                        if is_num {
                            count += w.number;
                        } else {
                            count += w.word;
                        }
                    }
                    WordType::Latin => {
                        if is_num {
                            // 字母 -> 数字，新 token
                            count += w.number;
                        }
                    }
                    WordType::Number => {
                        if !is_num {
                            // 数字 -> 字母，新 token
                            count += w.word;
                        }
                    }
                }
                current_type = new_type;
                continue;
            }

            // 5. 标点符号
            current_type = WordType::None;
            if is_math_symbol(ch) {
                count += w.math_symbol;
            } else if ch == '@' {
                count += w.at_sign;
            } else if is_url_delim(ch) {
                count += w.url_delim;
            } else {
                count += w.symbol;
            }
        }

        (count.ceil() as i32) + w.base_pad
    }

    /// 估算请求 token（兼容旧接口）
    pub fn estimate_request_tokens(&self, text: &str, model: &str) -> i32 {
        let provider = Self::detect_provider(model);
        self.estimate(text, &provider)
    }
}

/// 判断是否为 CJK 字符
fn is_cjk(ch: char) -> bool {
    let cp = ch as u32;
    // CJK 统一汉字
    (0x4E00..=0x9FFF).contains(&cp) ||
    // CJK 扩展 A
    (0x3400..=0x4DBF).contains(&cp) ||
    // 日文平假名
    (0x3040..=0x309F).contains(&cp) ||
    // 日文片假名
    (0x30A0..=0x30FF).contains(&cp) ||
    // 韩文
    (0xAC00..=0xD7A3).contains(&cp)
}

/// 判断是否为 Emoji
fn is_emoji(ch: char) -> bool {
    let cp = ch as u32;
    // Emoticons
    (0x1F600..=0x1F64F).contains(&cp) ||
    // Misc Symbols and Pictographs
    (0x1F300..=0x1F5FF).contains(&cp) ||
    // Transport and Map Symbols
    (0x1F680..=0x1F6FF).contains(&cp) ||
    // Supplemental Symbols and Pictographs
    (0x1F900..=0x1F9FF).contains(&cp) ||
    // Symbols and Pictographs Extended-A
    (0x1FA00..=0x1FAFF).contains(&cp) ||
    // Misc Symbols
    (0x2600..=0x26FF).contains(&cp) ||
    // Dingbats
    (0x2700..=0x27BF).contains(&cp)
}

/// 判断是否为数学符号
fn is_math_symbol(ch: char) -> bool {
    let cp = ch as u32;
    // Mathematical Operators
    (0x2200..=0x22FF).contains(&cp) ||
    // Supplemental Mathematical Operators
    (0x2A00..=0x2AFF).contains(&cp) ||
    // Mathematical Alphanumeric Symbols
    (0x1D400..=0x1D7FF).contains(&cp)
}

/// 判断是否为 URL 分隔符
fn is_url_delim(ch: char) -> bool {
    matches!(ch, '/' | ':' | '?' | '&' | '=' | ';' | '#' | '%')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_provider() {
        assert_eq!(TokenEstimator::detect_provider("gpt-4"), Provider::OpenAI);
        assert_eq!(TokenEstimator::detect_provider("gpt-4o-mini"), Provider::OpenAI);
        assert_eq!(TokenEstimator::detect_provider("o1-preview"), Provider::OpenAI);
        assert_eq!(TokenEstimator::detect_provider("claude-3-opus"), Provider::Claude);
        assert_eq!(TokenEstimator::detect_provider("gemini-1.5-pro"), Provider::Gemini);
        assert_eq!(TokenEstimator::detect_provider("unknown-model"), Provider::Unknown);
    }

    #[test]
    fn test_estimate_english() {
        let estimator = TokenEstimator::new();
        // "Hello World" 大约 2-3 个 token
        let tokens = estimator.estimate("Hello World", &Provider::OpenAI);
        assert!(tokens >= 2 && tokens <= 4);
    }

    #[test]
    fn test_estimate_chinese() {
        let estimator = TokenEstimator::new();
        // "你好世界" 大约 2-4 个 token
        let tokens = estimator.estimate("你好世界", &Provider::OpenAI);
        assert!(tokens >= 2 && tokens <= 5);
    }

    #[test]
    fn test_estimate_mixed() {
        let estimator = TokenEstimator::new();
        let text = "Hello 你好 World 世界";
        let tokens = estimator.estimate(text, &Provider::OpenAI);
        assert!(tokens >= 4 && tokens <= 8);
    }

    #[test]
    fn test_estimate_empty() {
        let estimator = TokenEstimator::new();
        assert_eq!(estimator.estimate("", &Provider::OpenAI), 0);
    }

    #[test]
    fn test_estimate_emoji() {
        let estimator = TokenEstimator::new();
        let tokens = estimator.estimate("Hello 😀 World", &Provider::OpenAI);
        assert!(tokens >= 3);
    }

    #[test]
    fn test_provider_weights_differ() {
        let estimator = TokenEstimator::new();
        let text = "你好世界 Hello World";
        
        let openai_tokens = estimator.estimate(text, &Provider::OpenAI);
        let claude_tokens = estimator.estimate(text, &Provider::Claude);
        let gemini_tokens = estimator.estimate(text, &Provider::Gemini);
        
        // 不同厂商的估算结果应该不同
        assert_ne!(openai_tokens, claude_tokens);
        assert_ne!(openai_tokens, gemini_tokens);
    }
}
```

#### 3.3.2 集成到 prepare.rs

修改 `src/proxy/prepare.rs`：

```rust
use crate::stats::token_estimator::TokenEstimator;

/// 估算 token 数量（新版本，支持多厂商）
pub(super) fn estimate_tokens(text: &str) -> i32 {
    if text.is_empty() {
        return 0;
    }
    // 默认使用 OpenAI 权重
    let estimator = TokenEstimator::new();
    estimator.estimate(text, &crate::stats::token_estimator::Provider::OpenAI)
}

/// 估算 token 数量（指定模型）
pub(super) fn estimate_tokens_for_model(text: &str, model: &str) -> i32 {
    if text.is_empty() {
        return 0;
    }
    let estimator = TokenEstimator::new();
    estimator.estimate_request_tokens(text, model)
}
```

---

## 4. 实施计划

### 4.1 阶段划分

| 阶段 | 任务 | 优先级 | 预计工作量 |
|------|------|--------|------------|
| 阶段 1 | JSON 日志脱敏 | P1 | 2 小时 |
| 阶段 2 | Token 估算精度提升 | P1 | 3 小时 |
| 阶段 3 | 三态熔断器 + 指数退避 | P1 | 4 小时 |

### 4.2 文件变更清单

**阶段 1（日志脱敏）**：
- 新增：`src/stats/redaction.rs`
- 修改：`src/stats/mod.rs`（导出模块）
- 修改：`src/proxy/execute.rs`（使用脱敏）
- 修改：`src/config.rs`（添加配置项）

**阶段 2（Token 估算）**：
- 新增：`src/stats/token_estimator.rs`
- 修改：`src/stats/mod.rs`（导出模块）
- 修改：`src/proxy/prepare.rs`（使用新估算器）
- 修改：`src/proxy/execute.rs`（使用新估算器）

**阶段 3（熔断器）**：
- 新增：`src/proxy/circuit.rs`
- 修改：`src/proxy/mod.rs`（导出模块）
- 修改：`src/proxy/state.rs`（集成熔断器）
- 修改：`src/proxy/execute.rs`（传递 key_hint）
- 修改：`src/config.rs`（添加配置项）

### 4.3 测试策略

**单元测试**：
- `redaction.rs`：脱敏逻辑、保护字段、截断
- `token_estimator.rs`：各厂商权重、字符类型识别
- `circuit.rs`：状态转换、指数退避、试探机制

**集成测试**：
- 端到端请求流程中验证脱敏生效
- 验证熔断器在连续失败时正确触发
- 验证 Token 估算与实际 usage 对比

### 4.4 向后兼容

- 所有新功能默认启用，但可通过配置关闭
- 数据库 schema 无变更
- API 接口无变更
- 配置文件新增项有默认值

---

## 附录

### A. 参考项目链接

- [Octopus](https://github.com/bestruirui/octopus) - 熔断器实现
- [New API](https://github.com/QuantumNous/new-api) - Token 估算
- [Sub2API](https://github.com/Wei-Shaw/sub2api) - 日志脱敏

### B. 相关文档

- [用户指南](./user-guide.md)
- [安装指南](./installation.md)
- [备份格式](./backup-format.md)
