-- Galaxy Router 初始 Schema（合并所有历史迁移）
-- 用于 sqlx-cli 首次迁移 V0

-- 管理员用户
CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- 上游渠道
CREATE TABLE IF NOT EXISTS channels (
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
    custom_headers TEXT NOT NULL DEFAULT '[]',
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- API Key
CREATE TABLE IF NOT EXISTS api_keys (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    api_key TEXT NOT NULL UNIQUE,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    supported_models TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- 模型分组
CREATE TABLE IF NOT EXISTS groups (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    match_regex TEXT,
    retry_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    max_retries INTEGER NOT NULL DEFAULT 3,
    first_token_timeout_secs INTEGER NOT NULL DEFAULT 30,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- 分组子项
CREATE TABLE IF NOT EXISTS group_items (
    id TEXT PRIMARY KEY,
    group_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    model_name TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 1,
    weight INTEGER NOT NULL DEFAULT 100,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(group_id, channel_id, model_name)
);

-- 模型信息（定价 + 元数据）
CREATE TABLE IF NOT EXISTS model_info (
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

-- 使用日志
CREATE TABLE IF NOT EXISTS usage_logs (
    id TEXT PRIMARY KEY,
    api_key_id TEXT,
    channel_id TEXT,
    group_id TEXT,
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
    request_content TEXT,
    response_content TEXT,
    is_stream BOOLEAN NOT NULL DEFAULT FALSE,
    upstream_key_hint TEXT,
    user_agent TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- 每日统计
CREATE TABLE IF NOT EXISTS usage_daily (
    id TEXT PRIMARY KEY,
    date TEXT NOT NULL,
    api_key_id TEXT,
    channel_id TEXT,
    group_id TEXT,
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
    UNIQUE(date, api_key_id, channel_id, group_id, model)
);

-- 系统设置
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    category TEXT NOT NULL DEFAULT 'general',
    value TEXT NOT NULL,
    description TEXT,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- 索引
CREATE INDEX IF NOT EXISTS idx_usage_logs_channel_id ON usage_logs(channel_id);
CREATE INDEX IF NOT EXISTS idx_usage_logs_api_key_id ON usage_logs(api_key_id);
CREATE INDEX IF NOT EXISTS idx_usage_logs_requested_model ON usage_logs(requested_model);
CREATE INDEX IF NOT EXISTS idx_usage_logs_created_at ON usage_logs(created_at);
CREATE INDEX IF NOT EXISTS idx_usage_logs_status_code ON usage_logs(status_code);

-- 默认设置
INSERT OR IGNORE INTO settings (key, category, value, description) VALUES
    ('scheduler.top_k', 'scheduler', '7', 'Top-K 候选数量'),
    ('scheduler.score_weights', 'scheduler', '{"priority":1.0,"load":1.0,"queue":0.7,"error_rate":0.8,"ttft":0.5}', '评分权重'),
    ('sticky_session.enabled', 'sticky_session', 'true', '是否启用粘性会话'),
    ('sticky_session.ttl_seconds', 'sticky_session', '3600', '会话保持时间（秒）'),
    ('proxy.enabled', 'proxy', 'false', '是否启用上游代理'),
    ('proxy.url', 'proxy', '', '代理地址（如 http://127.0.0.1:7890）');
