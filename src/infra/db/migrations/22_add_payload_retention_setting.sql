-- usage_payloads（请求/响应原文）保留天数：分层 retention 的 content 层
-- scheduler run_payload_cleanup 每日清旧 payload，保留 usage_logs 统计行
-- （content 大，短期清释放空间；统计行由 usage.retention_days 另控）
-- 默认 7 天
INSERT OR IGNORE INTO settings (key, category, value, description) VALUES
    ('usage.payload_retention_days', 'usage', '7', '请求/响应原文(usage_payloads)保留天数；scheduler 每日清理超过此天数的 payload，保留 usage_logs 统计行');
