-- 版本更新检查配置
-- 代理复用现有 proxy.enabled / proxy.url，无需新增
INSERT INTO settings (key, category, value, description) VALUES
    ('github.repo', 'update', 'gclm/galaxy-router', 'GitHub 仓库（owner/repo），用于检查版本更新'),
    ('update.mirror', 'update', '', '下载镜像前缀（如 https://ghfast.top/）；api.github.com 失败时走镜像下载 release-info.json，留空=不启用');
