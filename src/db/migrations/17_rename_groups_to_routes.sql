-- v1.1.1: groups → routes 全链路改名（表/列/索引），含插件配置项预设
-- 路径对齐约束 #5：migrations 不随重构迁移；不可回滚、不可修改已发布版本

-- 表改名
ALTER TABLE groups RENAME TO routes;
ALTER TABLE group_items RENAME TO route_items;

-- 列改名（先改表名再改列名）
ALTER TABLE route_items RENAME COLUMN group_id TO route_id;
ALTER TABLE usage_logs   RENAME COLUMN group_id TO route_id;
ALTER TABLE usage_daily  RENAME COLUMN group_id TO route_id;
-- ⚠️ allowed_groups 以独立 ALTER 加入（8_add_api_key_allowed_groups.sql），不在 initial_schema
ALTER TABLE api_keys     RENAME COLUMN allowed_groups TO allowed_routes;

-- 索引重建：SQLite RENAME COLUMN 不会自动重命名索引，旧索引名会名不副实
DROP INDEX IF EXISTS idx_usage_logs_group_id;
CREATE INDEX IF NOT EXISTS idx_usage_logs_route_id ON usage_logs(route_id);

-- 注：UNIQUE 约束（route_items 的 UNIQUE(group_id,channel_id,model_name)、
-- usage_daily 的 UNIQUE(date,api_key_id,channel_id,group_id,model)）随列改名自动跟随，无需重建。

-- 插件配置项（随 v1.1.1 迁移先建，v1.1.3 插件系统启用时读取；v1.1.1/2 期间存在但闲置）
INSERT OR IGNORE INTO settings (key, category, value, description) VALUES
    ('plugin.cch_rewrite', 'plugin', 'true', '清理 Claude Code cch 标记，提升缓存命中率'),
    ('plugin.tracking_removal', 'plugin', 'true', '清洗 Claude Code 隐私跟踪标记'),
    ('plugin.cache_key_injection', 'plugin', 'true', '注入 prompt_cache_key 实现粘性路由'),
    ('plugin.thinking_fix', 'plugin', 'true', '思维链处理：字段规范化 + 内容分离（承接 v1.0 reasoning 能力，默认开）'),
    ('plugin.master_switch', 'plugin', 'true', '全局总开关：false 时所有插件跳过（紧急回滚）');
