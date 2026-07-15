-- 添加缺失的索引
CREATE INDEX IF NOT EXISTS idx_api_keys_api_key ON api_keys(api_key);
CREATE INDEX IF NOT EXISTS idx_usage_logs_group_id ON usage_logs(group_id);
