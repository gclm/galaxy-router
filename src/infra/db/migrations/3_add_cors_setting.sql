-- 迁移 V3: 添加 CORS 跨域白名单配置
-- 参考 octopus 的动态 CORS 设计，支持从管理界面配置允许的跨域来源

INSERT OR IGNORE INTO settings (key, category, value, description) VALUES
    ('cors.allow_origins', 'cors', '*', '跨域白名单（逗号分隔域名，空=禁止跨域，*=允许所有，如 "https://example.com,http://localhost:3000"）');
