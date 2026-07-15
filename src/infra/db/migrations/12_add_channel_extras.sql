-- 渠道扩展字段（JSON 自由格式）
-- 当前用法：
--   thinking.extract_tags    (bool) 抽取 <think/> 标签到 reasoning_content
--   thinking.fix_signature  (bool) 修复 GLM-style signature 位置（content_block_start → signature_delta）
ALTER TABLE channels ADD COLUMN extras TEXT DEFAULT '{}' NOT NULL;

-- 迁移老数据：thinking_mode='normalize' → extras.thinking.{extract_tags,fix_signature}=true
-- 注意：SQLite 的 json_object() 不支持 true/false 字面量（会序列化为 1/0），
--      用 json() 包装布尔值确保是真正的 JSON bool
UPDATE channels SET extras = json_object(
    'thinking', json_object(
        'extract_tags', json('true'),
        'fix_signature', json('true')
    )
) WHERE thinking_mode = 'normalize';
