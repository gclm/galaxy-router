-- 渠道思维链规范化模式
-- 'normalize' = 启用 <think/> 标签抽取 + Anthropic signature 补发
-- NULL = 关闭（向后兼容）
ALTER TABLE channels ADD COLUMN thinking_mode TEXT DEFAULT NULL;
