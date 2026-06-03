-- 给 api_keys 增加 RPM / TPM 速率限制字段（0 = 不限制）
ALTER TABLE api_keys ADD COLUMN rate_limit_rpm INTEGER NOT NULL DEFAULT 0;
ALTER TABLE api_keys ADD COLUMN rate_limit_tpm INTEGER NOT NULL DEFAULT 0;
