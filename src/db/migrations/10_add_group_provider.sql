-- 分组添加厂家字段，空字符串表示自动识别
ALTER TABLE groups ADD COLUMN provider TEXT NOT NULL DEFAULT '';
