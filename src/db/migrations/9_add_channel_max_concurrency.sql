-- 渠道增加最大并发数（0=不限制）
ALTER TABLE channels ADD COLUMN max_concurrency INTEGER NOT NULL DEFAULT 0;
