-- 增加 per-channel 超时配置
ALTER TABLE channels ADD COLUMN timeout_secs INTEGER NOT NULL DEFAULT 300;
