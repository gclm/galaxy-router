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
const MAX_CONTENT_LENGTH: usize = 16 * 1024; // 16KB

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
#[allow(dead_code)]
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
        assert_eq!(value["max_tokens"], 100); // 保护字段不脱敏
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
    fn test_sanitize_json_content_empty() {
        assert_eq!(sanitize_json_content(""), "");
    }

    #[test]
    fn test_sanitize_json_content_non_json() {
        let content = "this is not json";
        assert_eq!(sanitize_json_content(content), content);
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

    #[test]
    fn test_extract_summary_invalid_json() {
        assert!(extract_summary("not json").is_none());
    }

    #[test]
    fn test_is_sensitive_key() {
        assert!(is_sensitive_key("authorization"));
        assert!(is_sensitive_key("Authorization"));
        assert!(is_sensitive_key("api_key"));
        assert!(is_sensitive_key("API_KEY"));
        assert!(is_sensitive_key("client_secret"));
        assert!(is_sensitive_key("password"));

        // 保护字段
        assert!(!is_sensitive_key("max_tokens"));
        assert!(!is_sensitive_key("prompt_tokens"));
        assert!(!is_sensitive_key("completion_tokens"));
        assert!(!is_sensitive_key("input_tokens"));
        assert!(!is_sensitive_key("output_tokens"));
        assert!(!is_sensitive_key("budget_tokens"));
    }

    #[test]
    fn test_nested_redaction() {
        let mut value = json!({
            "headers": {
                "Authorization": "Bearer sk-xxx",
                "Content-Type": "application/json"
            },
            "body": {
                "api_key": "sk-yyy",
                "model": "gpt-4"
            }
        });

        redact_value(&mut value);

        assert_eq!(value["headers"]["Authorization"], REDACTED);
        assert_eq!(value["headers"]["Content-Type"], "application/json");
        assert_eq!(value["body"]["api_key"], REDACTED);
        assert_eq!(value["body"]["model"], "gpt-4");
    }

    #[test]
    fn test_array_redaction() {
        let mut value = json!({
            "items": [
                {"api_key": "sk-1", "name": "first"},
                {"api_key": "sk-2", "name": "second"}
            ]
        });

        redact_value(&mut value);

        assert_eq!(value["items"][0]["api_key"], REDACTED);
        assert_eq!(value["items"][0]["name"], "first");
        assert_eq!(value["items"][1]["api_key"], REDACTED);
        assert_eq!(value["items"][1]["name"], "second");
    }
}
