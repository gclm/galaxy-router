-- galaxy-router 全量初始 schema（压缩自迁移 1-22）
-- 不考虑兼容性：新部署用此单文件；已部署 DB 需重建（_sqlx_migrations version 不匹配）
-- 由「干净 DB 跑完 1-22 → .schema + settings seed」合并，ALTER 累积列已内联到 CREATE TABLE

-- ===== 表 =====

CREATE TABLE users (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE channels (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    api_keys TEXT NOT NULL DEFAULT '[]',
    endpoints TEXT NOT NULL DEFAULT '[]',
    models TEXT NOT NULL DEFAULT '[]',
    rate_limit_rpm INTEGER,
    rate_limit_tpm INTEGER,
    failure_threshold INTEGER NOT NULL DEFAULT 3,
    blacklist_minutes INTEGER NOT NULL DEFAULT 10,
    concurrency INTEGER NOT NULL DEFAULT 10,
    timeout_secs INTEGER NOT NULL DEFAULT 300,
    max_concurrency INTEGER NOT NULL DEFAULT 0,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE api_keys (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    api_key TEXT NOT NULL UNIQUE,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    supported_models TEXT NOT NULL DEFAULT '',
    rate_limit_rpm INTEGER NOT NULL DEFAULT 0,
    rate_limit_tpm INTEGER NOT NULL DEFAULT 0,
    allowed_routes TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE routes (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    provider TEXT NOT NULL DEFAULT '',
    match_regex TEXT,
    retry_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    first_token_timeout_secs INTEGER NOT NULL DEFAULT 30,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE route_items (
    id TEXT PRIMARY KEY,
    route_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    model_name TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 1,
    weight INTEGER NOT NULL DEFAULT 100,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(route_id, channel_id, model_name)
);

CREATE TABLE model_info (
    id TEXT PRIMARY KEY,
    model TEXT NOT NULL UNIQUE,
    provider TEXT NOT NULL DEFAULT '',
    mode TEXT NOT NULL DEFAULT 'chat',
    input_price REAL,
    output_price REAL,
    cache_read_price REAL,
    cache_creation_price REAL,
    max_input_tokens INTEGER,
    max_output_tokens INTEGER,
    supports_function_calling BOOLEAN,
    supports_reasoning BOOLEAN,
    supports_vision BOOLEAN,
    supports_pdf_input BOOLEAN,
    supports_prompt_caching BOOLEAN,
    supports_system_messages BOOLEAN,
    supports_tool_choice BOOLEAN,
    source TEXT NOT NULL DEFAULT 'remote',
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE usage_logs (
    id TEXT PRIMARY KEY,
    api_key_id TEXT,
    channel_id TEXT,
    route_id TEXT,
    requested_model TEXT NOT NULL,
    actual_model TEXT,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
    cost REAL,
    latency_ms INTEGER,
    ttft_ms INTEGER,
    attempts TEXT,
    status_code INTEGER,
    error_message TEXT,
    endpoint_type TEXT,
    request_type TEXT NOT NULL DEFAULT 'passthrough',
    is_stream BOOLEAN NOT NULL DEFAULT FALSE,
    upstream_key_hint TEXT,
    user_agent TEXT,
    request_id TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE usage_daily (
    id TEXT PRIMARY KEY,
    date TEXT NOT NULL,
    api_key_id TEXT,
    channel_id TEXT,
    route_id TEXT,
    model TEXT NOT NULL,
    request_count INTEGER NOT NULL DEFAULT 0,
    success_count INTEGER NOT NULL DEFAULT 0,
    failure_count INTEGER NOT NULL DEFAULT 0,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
    total_cost REAL NOT NULL DEFAULT 0,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(date, api_key_id, channel_id, route_id, model)
);

CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    category TEXT NOT NULL DEFAULT 'general',
    value TEXT NOT NULL,
    description TEXT,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE budget_limits (
    id TEXT PRIMARY KEY,
    api_key_id TEXT NOT NULL,
    monthly_limit_usd REAL NOT NULL DEFAULT 0,
    daily_limit_usd REAL NOT NULL DEFAULT 0,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (api_key_id) REFERENCES api_keys(id) ON DELETE CASCADE,
    UNIQUE(api_key_id)
);

CREATE TABLE usage_payloads (
    log_id TEXT PRIMARY KEY REFERENCES usage_logs(id) ON DELETE CASCADE,
    request_content TEXT,
    response_content TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- ===== 索引 =====

CREATE INDEX idx_usage_logs_channel_id ON usage_logs(channel_id);
CREATE INDEX idx_usage_logs_api_key_id ON usage_logs(api_key_id);
CREATE INDEX idx_usage_logs_requested_model ON usage_logs(requested_model);
CREATE INDEX idx_usage_logs_created_at ON usage_logs(created_at);
CREATE INDEX idx_usage_logs_status_code ON usage_logs(status_code);
CREATE INDEX idx_usage_logs_route_id ON usage_logs(route_id);
CREATE INDEX idx_api_keys_api_key ON api_keys(api_key);
CREATE INDEX idx_budget_limits_api_key_id ON budget_limits(api_key_id);
CREATE INDEX idx_usage_payloads_created_at ON usage_payloads(created_at);

-- ===== settings seed（16 条默认值）=====

INSERT INTO settings (key, category, value, description) VALUES ('cors.allow_origins', 'cors', '*', '跨域白名单（逗号分隔域名，空=禁止跨域，*=允许所有）');
INSERT INTO settings (key, category, value, description) VALUES ('plugin.cache_key_injection', 'plugin', 'false', '注入 prompt_cache_key 实现粘性路由');
INSERT INTO settings (key, category, value, description) VALUES ('plugin.cch_rewrite', 'plugin', 'false', '清理 Claude Code cch 标记，提升缓存命中率');
INSERT INTO settings (key, category, value, description) VALUES ('plugin.master_switch', 'plugin', 'true', '全局总开关：false 时所有插件跳过（紧急回滚）');
INSERT INTO settings (key, category, value, description) VALUES ('plugin.thinking_fix', 'plugin', 'true', '思维链处理：字段规范化 + 内容分离（默认开）');
INSERT INTO settings (key, category, value, description) VALUES ('plugin.tracking_removal', 'plugin', 'false', '清洗 Claude Code 隐私跟踪标记');
INSERT INTO settings (key, category, value, description) VALUES ('proxy.enabled', 'proxy', 'false', '是否启用上游代理');
INSERT INTO settings (key, category, value, description) VALUES ('proxy.url', 'proxy', '', '代理地址（如 http://127.0.0.1:7890）');
INSERT INTO settings (key, category, value, description) VALUES ('scheduler.score_weights', 'scheduler', '{"priority":1.0,"load":1.0,"queue":0.7,"error_rate":0.8,"ttft":0.5}', '评分权重');
INSERT INTO settings (key, category, value, description) VALUES ('scheduler.top_k', 'scheduler', '7', 'Top-K 候选数量');
INSERT INTO settings (key, category, value, description) VALUES ('sticky_session.enabled', 'sticky_session', 'true', '是否启用粘性会话');
INSERT INTO settings (key, category, value, description) VALUES ('sticky_session.ttl_seconds', 'sticky_session', '3600', '会话保持时间（秒）');
INSERT INTO settings (key, category, value, description) VALUES ('github.repo', 'update', 'gclm/galaxy-router', 'GitHub 仓库（owner/repo），用于检查版本更新');
INSERT INTO settings (key, category, value, description) VALUES ('update.mirror', 'update', 'https://ghfast.top/', '下载镜像前缀；api.github.com 失败时走镜像，留空=不启用');
INSERT INTO settings (key, category, value, description) VALUES ('usage.record_content', 'usage', 'true', '是否记录请求/响应原文到 usage_payloads（关闭=新请求不写 payload）');
INSERT INTO settings (key, category, value, description) VALUES ('usage.retention_days', 'usage', '30', '请求日志保留天数（scheduler 每日清理超过此天数的 usage_logs，级联删 usage_payloads）');
INSERT INTO settings (key, category, value, description) VALUES ('usage.payload_retention_days', 'usage', '7', '请求/响应原文(usage_payloads)保留天数；scheduler 每日清理超过此天数的 payload，保留 usage_logs 统计行');
