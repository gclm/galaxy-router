-- API Key 增加允许访问的分组列表（逗号分隔，空=允许所有）
ALTER TABLE api_keys ADD COLUMN allowed_groups TEXT NOT NULL DEFAULT '';
