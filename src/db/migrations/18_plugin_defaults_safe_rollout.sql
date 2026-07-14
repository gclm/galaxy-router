-- Step C：插件开关灰度上线。
-- migration 17 已把 cch/tracking/cache_key 默认置 true（不可改已发布版本），
-- 本迁移翻为 false，避免上线即对所有 Anthropic/Responses 流量改写（正则未校准前风险）。
-- master_switch / thinking_fix 保持 true：master_switch 作刹车（admin 开插件后立即生效），
-- thinking_fix 留待 thinking 插件落地（Step C 不消费）。
UPDATE settings SET value = 'false' WHERE key IN (
    'plugin.cch_rewrite',
    'plugin.tracking_removal',
    'plugin.cache_key_injection'
);
