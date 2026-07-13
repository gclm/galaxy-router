//! 时间工具（v1.1.2 从 metrics/query/mod.rs 抽出）。
//!
//! 被 admin handler（api_keys/channels/routes）与 repository/usage_repository
//! 跨层复用，放 crate 顶层 util 避免任何层向下依赖。

/// 生成 SQLite datetime() 修饰符，如 "+8 hours" 或 "-5 hours"
pub fn tz_modifier(offset: i32) -> String {
    assert!(
        (-12..=14).contains(&offset),
        "时区偏移量超出合理范围: {}",
        offset
    );
    if offset >= 0 {
        format!("+{} hours", offset)
    } else {
        format!("-{} hours", offset.abs())
    }
}

/// 生成当前本地时间字符串（用于 INSERT）
pub fn now_local_str(offset: i32) -> String {
    (chrono::Utc::now() + chrono::Duration::hours(offset as i64))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}
