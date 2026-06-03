# 配置格式设计

## 配置分层

| 层级 | 存储方式 | 内容 | 更新方式 |
|------|---------|------|---------|
| 基础配置 | TOML 文件 | 服务器、数据库、日志 | 重启生效 |
| 业务配置 | SQLite 数据库 | 渠道、分组、定价、模型映射 | Web 面板实时生效 |

## TOML 基础配置

**文件路径**: `./config.toml`（可通过 `GALAXY_PROXY_CONFIG` 环境变量覆盖）

```toml
[server]
host = "127.0.0.1"
port = 8080

[database]
path = "data/galaxy.db"

[logging]
level = "info"              # trace | debug | info | warn | error
format = "compact"          # compact | json
file = false                # 是否输出到文件
file_path = "logs/galaxy.log"

[auth]
jwt_secret = ""             # 首次运行自动生成
token_expiry_hours = 24
```

## 数据库 settings 表

运行时可调的配置存储在数据库中，通过 Web 面板管理。

```sql
CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    category TEXT NOT NULL DEFAULT 'general',
    value TEXT NOT NULL,
    description TEXT,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
```

**默认配置**（首次运行时插入）：

| key | category | 默认值 | 说明 |
|-----|----------|--------|------|
| `scheduler.top_k` | scheduler | `7` | Top-K 候选数量 |
| `scheduler.score_weights` | scheduler | `{"priority":1.0,...}` | 评分权重 |
| `sticky_session.enabled` | sticky_session | `true` | 是否启用粘性会话 |
| `sticky_session.ttl_seconds` | sticky_session | `3600` | 会话保持时间（秒） |
| `stats.log_detail_mode` | stats | `failures_only` | 日志模式 |
| `stats.cost.source` | stats | `models.dev` | 成本数据源 |
| `stats.cost.refresh_interval_hours` | stats | `24` | 刷新间隔（小时） |

**前端按 category 分组显示设置项。**

## 数据库 Schema

### channels 表

```sql
CREATE TABLE channels (
    id TEXT PRIMARY KEY,                          -- UUID v7
    name TEXT NOT NULL UNIQUE,
    api_keys TEXT NOT NULL DEFAULT '[]',        -- JSON 数组（UpstreamApiKey[]），如 [{"key":"sk-xxx","note":"","enabled":true}]
    endpoints TEXT NOT NULL DEFAULT '[]',       -- JSON 数组（EndpointConfig[]），含 type/base_url/enabled
    models TEXT NOT NULL DEFAULT '[]',          -- JSON 字符串数组，如 ["gpt-4o","claude-3-5-sonnet"]
    rate_limit_rpm INTEGER,
    rate_limit_tpm INTEGER,
    failure_threshold INTEGER NOT NULL DEFAULT 3,
    blacklist_minutes INTEGER NOT NULL DEFAULT 10,
    concurrency INTEGER NOT NULL DEFAULT 10,
    custom_headers TEXT NOT NULL DEFAULT '[]',  -- JSON 数组（CustomHeader[]），如 [{"key":"X-Trace-Id","value":"..."}]
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
```

**字段说明**:
- `api_keys`: 多 Key 时按 `AtomicU64` 轮询（`api_key_attempts`）；元素为 `UpstreamApiKey { key, note, enabled }`
- `endpoints`: 端点配置数组；元素为 `EndpointConfig { type, base_url, enabled }`，`type` 取值见下表
- `models`: 渠道支持的模型名列表；模型映射在分组层 `group_items.model_name` 实现，channel 层不做改写

**端点类型与路径映射**（`EndpointType::path()`）:

| `type` | 端点路径 |
|--------|---------|
| `openai_chat` | `{base_url}/chat/completions` |
| `openai_response` | `{base_url}/responses` |
| `anthropic` | `{base_url}/messages` |
| `gemini` | `{base_url}/models/{model}:generateContent` |
| `openai_embedding` | `{base_url}/embeddings` |
| `openai_images` | `{base_url}/images/generations` |

### api_keys 表（客户端侧 Key，用于访问 Proxy）

```sql
CREATE TABLE api_keys (
    id TEXT PRIMARY KEY,                    -- UUID v7
    name TEXT NOT NULL,
    api_key TEXT NOT NULL UNIQUE,           -- 客户端使用的 Key，格式 `gp-<uuid v7>`
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    supported_models TEXT NOT NULL DEFAULT '',  -- 逗号分隔的分组/模型白名单，空字符串=不限制
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
```

**说明**:
- `supported_models` 由 `parse_supported_models` 解析；为空表示该 Key 可访问所有模型
- 写入 `enabled` / `name` / `supported_models` 变更需同步调用 `ApiKeyCache::invalidate(api_key_value)`

### groups 表

```sql
CREATE TABLE groups (
    id TEXT PRIMARY KEY,              -- UUID v7
    name TEXT NOT NULL UNIQUE,        -- 对外暴露的模型名（请求 model 精确匹配 / 正则匹配）
    match_regex TEXT,                 -- 可选：正则匹配（命中后映射到该分组）
    retry_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    max_retries INTEGER NOT NULL DEFAULT 3,
    first_token_timeout_secs INTEGER NOT NULL DEFAULT 30,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
```

**说明**:
- 无 `mode` 字段；分组内渠道选择走 `select_group_item`（Top-K=3 加权随机 + `lb_state` 评分）
- `match_regex` 为可空正则，命中时该分组生效
- 缓存：命中通过 `ProxyCache.set_group`；删除/更新走 `invalidate_all_groups`

### group_items 表

```sql
CREATE TABLE group_items (
    id TEXT PRIMARY KEY,                        -- UUID v7
    group_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    model_name TEXT NOT NULL,                   -- 转发到上游时实际使用的模型名（= 渠道支持模型的别名/精确名）
    priority INTEGER NOT NULL DEFAULT 1,
    weight INTEGER NOT NULL DEFAULT 100,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(group_id, channel_id, model_name)
);
```

**说明**:
- 无外键约束（sqlx-cli 模式不依赖外键）；删除渠道/分组由应用层显式处理
- 唯一约束保证 `(group, channel, model_name)` 不重复

### model_info 表

```sql
CREATE TABLE model_info (
    id TEXT PRIMARY KEY,                          -- UUID v7
    model TEXT NOT NULL UNIQUE,
    provider TEXT NOT NULL DEFAULT '',
    mode TEXT NOT NULL DEFAULT 'chat',           -- chat | embedding | image 等
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
    source TEXT NOT NULL DEFAULT 'remote',       -- remote（models.dev）| manual
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
```

**说明**:
- 替代旧文档中的 `model_pricing`；含定价 + 模型能力元数据
- 远程刷新由 `ModelRegistry` + `PricingRefresher` 处理，写入 `source = "remote"`

## API 端点设计

### 代理 API（客户端使用，无需 /api 前缀）

客户端只需配置 `http://ip:port`，SDK 自动拼接 `/v1/*` 路径。

| 端点 | 方法 | 说明 |
|------|------|------|
| `/v1/chat/completions` | POST | OpenAI Chat Completions |
| `/v1/responses` | POST | OpenAI Responses |
| `/v1/messages` | POST | Anthropic Messages |
| `/v1/embeddings` | POST | OpenAI Embedding |
| `/v1/images/generations` | POST | OpenAI Images |
| `/v1/models` | GET | 获取可用模型列表 |

### 管理 API（Web 面板使用）

| 端点 | 方法 | 说明 |
|------|------|------|
| `/api/v1/admin/auth/setup` | POST | 初始化管理员 |
| `/api/v1/admin/auth/login` | POST | 登录 |
| `/api/v1/admin/auth/password` | PUT | 修改密码 |
| `/api/v1/admin/channels` | GET/POST | 渠道列表/创建 |
| `/api/v1/admin/channels/:id` | GET/PUT/DELETE | 渠道详情/更新/删除 |
| `/api/v1/admin/groups` | GET/POST | 分组列表/创建 |
| `/api/v1/admin/groups/:id` | GET/PUT/DELETE | 分组详情/更新/删除 |
| `/api/v1/admin/groups/:id/items` | POST | 添加分组项 |
| `/api/v1/admin/api-keys` | GET/POST | API Key 列表/创建 |
| `/api/v1/admin/api-keys/:id` | DELETE | 删除 API Key |
| `/api/v1/admin/stats/overview` | GET | 统计概览 |
| `/api/v1/admin/stats/daily` | GET | 按天统计 |
| `/api/v1/admin/stats/models` | GET | 按模型统计 |
| `/api/v1/admin/pricing` | GET/PUT | 定价管理 |

## Web 前端技术栈

| 技术 | 版本 | 说明 |
|------|------|------|
| React | 18+ | UI 框架 |
| TypeScript | 5+ | 类型安全 |
| Vite | 5+ | 构建工具 |
| Tailwind CSS | 3+ | 样式 |
| shadcn/ui | — | 组件库（参考 AxonHub） |
| TanStack Query | — | 数据获取 |
| React Router | — | 路由 |

## 前端目录结构

```
frontend/
├── src/
│   ├── main.tsx
│   ├── App.tsx
│   ├── api/                    # API 客户端
│   │   ├── channels.ts
│   │   ├── groups.ts
│   │   ├── stats.ts
│   │   └── pricing.ts
│   ├── components/             # 通用组件
│   │   ├── ui/                 # shadcn/ui 组件
│   │   └── layout/             # 布局组件
│   ├── pages/                  # 页面
│   │   ├── Dashboard.tsx       # 统计概览
│   │   ├── Channels.tsx        # 渠道管理
│   │   ├── Groups.tsx          # 分组管理
│   │   └── Settings.tsx        # 设置
│   └── types/                  # TypeScript 类型
│       └── index.ts
├── index.html
├── package.json
├── tsconfig.json
├── tailwind.config.js
└── vite.config.ts
```

## 嵌入式部署

前端构建产物嵌入到 Rust 二进制中：

```rust
// 使用 rust-embed 或 include_dir
#[derive(RustEmbed)]
#[folder = "frontend/dist"]
struct Frontend;

// axum 路由
let app = Router::new()
    .nest("/admin/api", admin_api_routes)
    .fallback(serve_frontend);
```

## 模型匹配优先级

请求的 `model` 字段匹配分组的规则（见 `ProxyState::select_channel_inner`）：

1. **精确匹配优先**：`groups.name` == model → 直接使用该分组
2. **正则匹配**：按 `groups.match_regex` 匹配，多个匹配时取第一个
3. **匹配顺序**：精确 > 正则

**示例**:
- 请求 `model = "claude-sonnet-4-20250514"`
- 匹配顺序：`groups.name = "claude-sonnet-4-20250514"` > `groups.match_regex = "^claude-.*"`
- 命中分组后，由 `group_items.model_name` 决定实际转发到上游的模型名（渠道层不做模型名改写）

## 渠道多 Key 选择策略

当一个渠道有多个 Key 时，使用**轮询（Round Robin）**选择，避免单个 Key 触发速率限制。

## 配置热更新机制

Web 面板写入 SQLite 后，内存缓存通过**写穿透**方式更新：

1. Web API 写入 SQLite
2. 写入成功后，更新内存缓存（Mutex/RwLock 保护的 HashMap）
3. 下次请求立即使用新配置

**注意**: TOML 基础配置需要重启才能生效。

## 健康探测策略（P2）

| 项目 | 说明 |
|------|------|
| 探测端点 | `/v1/models`（轻量级，不消耗 Token） |
| 探测间隔 | 默认 60 秒 |
| 判断标准 | HTTP 状态码 + 响应时间 |
| 失败阈值 | 连续 3 次失败标记为不健康 |
| 恢复条件 | 连续 2 次成功恢复为健康 |

## 成本计算降级策略

| 场景 | 行为 |
|------|------|
| 启动时 models.dev 不可达 | 使用本地 `model_pricing` 表，日志警告 |
| 运行时刷新失败 | 继续使用上次成功拉取的数据 |
| 本地覆盖优先级 | `source = 'manual'` 的记录优先于 `source = 'models.dev'` |
