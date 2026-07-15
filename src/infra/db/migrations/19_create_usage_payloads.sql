-- Step E: 响应内容记录拆表 + usage.record_content 开关
-- 大 TEXT 移出 usage_logs 避免拖慢其 9+ 分页/聚合查询；详情按需 JOIN usage_payloads

CREATE TABLE IF NOT EXISTS usage_payloads (
    log_id TEXT PRIMARY KEY REFERENCES usage_logs(id) ON DELETE CASCADE,
    request_content TEXT,
    response_content TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_usage_payloads_created_at ON usage_payloads(created_at);

-- 回填：把 usage_logs 现有 content 搬到 usage_payloads（migration 20 会从 usage_logs 删这两列）
INSERT OR IGNORE INTO usage_payloads (log_id, request_content, response_content, created_at)
SELECT id, request_content, response_content, created_at FROM usage_logs
WHERE request_content IS NOT NULL OR response_content IS NOT NULL;

-- 记录开关（默认开；关闭=新请求不写 payload，历史不受影响）
INSERT OR IGNORE INTO settings (key, category, value, description) VALUES
    ('usage.record_content', 'usage', 'true', '是否记录请求/响应原文到 usage_payloads（关闭=新请求不写 payload，历史不受影响）');
