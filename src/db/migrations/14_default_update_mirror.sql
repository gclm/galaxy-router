-- 默认下载镜像设为 ghfast.top（仅当用户未自定义/留空时）
UPDATE settings SET value = 'https://ghfast.top/' WHERE key = 'update.mirror' AND value = '';
