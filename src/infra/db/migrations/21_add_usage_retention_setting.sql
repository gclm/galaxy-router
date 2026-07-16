-- usage 日志保留天数可配置（缺口加固：retention 不再硬编码 90 天）
-- scheduler_task.run_log_cleanup 每日读此值，清理超过 N 天的 usage_logs
-- （usage_payloads.log_id ON DELETE CASCADE，删日志自动删 payload）
-- 默认 30 天（比旧硬编码 90 更激进，配合 content 拆表缓解 DB 膨胀）
INSERT OR IGNORE INTO settings (key, category, value, description) VALUES
    ('usage.retention_days', 'usage', '30', '请求日志保留天数（scheduler 每日清理超过此天数的 usage_logs，级联删 usage_payloads）');
