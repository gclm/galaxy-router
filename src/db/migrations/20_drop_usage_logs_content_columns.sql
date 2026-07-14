-- Step E: 从 usage_logs 删除 content 列（已回填到 usage_payloads，见 migration 19）
-- SQLite 3.35+ 支持 DROP COLUMN（sqlx 0.9 bundled libsqlite3-sys 满足）

ALTER TABLE usage_logs DROP COLUMN request_content;
ALTER TABLE usage_logs DROP COLUMN response_content;
