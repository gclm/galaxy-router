-- 废弃字段清理（task 2026-07-01-智能调度打分重构）
--
-- 1. groups.max_retries：调度器改为遍历全部候选（外层 for candidate），不再读该字段
-- 2. channels.custom_headers：已迁移到 endpoints[].headers（commit 5438140）
-- 3. channels.extras：已迁移到 endpoints[].extras（commit 5438140）
--
-- 三列在代码中已无引用（admin/relay/backup SQL 均已清理），安全 DROP。

ALTER TABLE groups DROP COLUMN max_retries;

ALTER TABLE channels DROP COLUMN custom_headers;

ALTER TABLE channels DROP COLUMN extras;
