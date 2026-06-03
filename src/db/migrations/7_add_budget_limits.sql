-- 预算限制表：per-API-key 月/日额度
CREATE TABLE IF NOT EXISTS budget_limits (
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
CREATE INDEX IF NOT EXISTS idx_budget_limits_api_key_id ON budget_limits(api_key_id);
