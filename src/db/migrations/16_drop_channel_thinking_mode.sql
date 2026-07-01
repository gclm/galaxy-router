-- 废弃字段清理（task 2026-07-01-渠道测试交互重构）
--
-- channels.thinking_mode：迁移 11 加的思维链模式列（'normalize' / NULL）。
-- 迁移 12 已将 thinking_mode='normalize' 的语义迁移到 endpoints[].extras.thinking；
-- 本次重构删除思维链运行时处理（thinking_normalizer 模块 + EndpointConfig.extras 字段），
-- thinking_mode 列在代码中已无引用（admin/relay/backup SQL 均不读），安全 DROP。

ALTER TABLE channels DROP COLUMN thinking_mode;
