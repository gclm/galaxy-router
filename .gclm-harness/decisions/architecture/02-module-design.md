# 模块划分

## 目录结构

```
galaxy-router/
├── src/
│   ├── main.rs                     # 入口，CLI 参数解析，服务启动
│   ├── lib.rs                      # 库入口，模块导出
│   ├── config.rs                   # TOML 配置加载与解析
│   ├── static_assets.rs            # 静态资源嵌入
│   │
│   ├── api/                        # HTTP 层
│   │   ├── mod.rs
│   │   ├── router.rs               # axum Router 定义
│   │   ├── response.rs             # 统一响应格式
│   │   ├── handlers/
│   │   │   ├── mod.rs
│   │   │   ├── proxy/              # 代理 API（/v1/*）
│   │   │   │   ├── mod.rs
│   │   │   │   ├── chat.rs         # /v1/chat/completions
│   │   │   │   ├── responses.rs    # /v1/responses
│   │   │   │   ├── messages.rs     # /v1/messages
│   │   │   │   ├── embeddings.rs   # /v1/embeddings
│   │   │   │   ├── images.rs       # /v1/images/generations
│   │   │   │   └── models.rs       # /v1/models
│   │   │   └── admin/              # 管理 API（/api/v1/admin/*）
│   │   │       ├── mod.rs
│   │   │       ├── auth.rs         # 认证 API
│   │   │       ├── channels.rs     # 渠道管理 API
│   │   │       │   ├── crud.rs     # 渠道 CRUD
│   │   │       │   ├── probe.rs    # 渠道探测
│   │   │       │   └── types.rs    # 渠道类型定义
│   │   │       ├── groups.rs       # 分组管理 API
│   │   │       ├── api_keys.rs     # API Key 管理 API
│   │   │       ├── stats.rs        # 统计查询 API
│   │   │       ├── settings.rs     # 系统设置 API
│   │   │       ├── backup.rs       # 数据备份 API
│   │   │       ├── fetch_models.rs # 获取上游模型 API
│   │   │       ├── model_info.rs   # 模型信息 API
│   │   │       └── system_info.rs  # 系统信息 API
│   │   └── middleware/
│   │       ├── mod.rs
│   │       └── cors.rs             # CORS 中间件
│   │
│   ├── auth/                       # 认证模块
│   │   ├── mod.rs
│   │   ├── password.rs             # 密码哈希
│   │   └── jwt.rs                  # JWT 认证
│   │
│   ├── protocol/                   # 协议转换层
│   │   ├── mod.rs
│   │   ├── model.rs                # 统一内部模型（LlmRequest, LlmResponse）
│   │   ├── inbound.rs              # 入站转换器 trait
│   │   ├── outbound.rs             # 出站转换器 trait
│   │   ├── openai_chat.rs          # OpenAI Chat 协议处理
│   │   ├── openai_responses.rs     # OpenAI Responses 协议处理
│   │   └── anthropic.rs            # Anthropic Messages 协议处理
│   │
│   ├── proxy/                      # 代理核心
│   │   ├── mod.rs                  # 主入口，handle_proxy_request
│   │   ├── cache.rs                # ProxyCache，模型索引，渠道缓存
│   │   ├── channel.rs              # 渠道选择逻辑
│   │   ├── selection.rs            # 模型选择策略
│   │   ├── scheduler.rs            # API Key 轮询调度
│   │   ├── circuit.rs              # 三态熔断器
│   │   ├── prepare.rs              # 请求准备（认证、Header 构建）
│   │   ├── execute.rs              # 请求执行（流式/非流式）
│   │   ├── queue.rs                # 并发队列控制
│   │   ├── state.rs                # 代理状态管理
│   │   └── sse.rs                  # SSE 解析与字段处理
│   │
│   ├── stats/                      # 统计模块
│   │   ├── mod.rs
│   │   ├── model.rs                # 统计数据模型
│   │   ├── recorder.rs             # 请求记录
│   │   ├── token_estimator.rs      # Token 估算（兜底）
│   │   ├── pricing_refresher.rs    # 模型定价刷新
│   │   └── redaction.rs            # 日志脱敏
│   │
│   ├── db/                         # 数据库层
│   │   ├── mod.rs                  # SQLite 连接、迁移
│   │   └── ...                     # Schema 定义
│   │
├── frontend/                       # Web 管理面板
│   ├── src/
│   │   ├── main.tsx
│   │   ├── App.tsx
│   │   ├── api/                    # API 客户端
│   │   ├── components/             # 通用组件
│   │   ├── pages/                  # 页面
│   │   └── types/                  # TypeScript 类型
│   ├── index.html
│   ├── package.json
│   ├── tsconfig.json
│   └── vite.config.ts
│
├── config.toml                     # 基础配置文件
├── Cargo.toml
└── docs/                           # 项目文档
```

## 模块职责

| 模块 | 职责 | 实际文件 |
|------|------|----------|
| `api` | HTTP 路由、请求处理、中间件 | `router.rs`, `handlers/`, `middleware/` |
| `api::admin` | 管理 API（渠道/分组/统计/设置/备份） | `handlers/admin/*.rs` |
| `api::proxy` | 代理 API（/v1/* 端点） | `handlers/proxy/*.rs` |
| `auth` | 密码哈希、JWT 认证 | `password.rs`, `jwt.rs` |
| `protocol` | 协议转换（OpenAI Chat / Responses / Anthropic） | `inbound.rs`, `outbound.rs`, `openai_chat.rs`, `openai_responses.rs`, `anthropic.rs` |
| `proxy` | 代理核心（缓存、选路、执行、熔断、排队） | `mod.rs`, `cache.rs`, `channel.rs`, `selection.rs`, `scheduler.rs`, `execute.rs`, `circuit.rs` |
| `stats` | 统计记录、Token 估算、定价刷新、日志脱敏 | `recorder.rs`, `token_estimator.rs`, `pricing_refresher.rs` |
| `db` | 数据库连接、迁移 | `mod.rs` |
| `config` | TOML 配置加载 | `config.rs` |
| `frontend` | Web 管理面板（React） | `frontend/` |

## 请求处理流程

```
客户端请求
    │
    ▼
api::handlers::proxy::*    # HTTP 处理器入口（chat/responses/messages）
    │
    ▼
proxy::mod::handle_proxy_request  # 统一代理入口
    │
    ▼
proxy::cache               # 获取渠道缓存、模型索引
    │
    ▼
proxy::selection            # 选择渠道（模型匹配）
    │
    ▼
proxy::scheduler            # API Key 轮询调度
    │
    ▼
proxy::prepare              # 构建请求（认证、Header）
    │
    ▼
proxy::execute              # 执行请求（流式/非流式；必要时做协议转换）
    │
    ▼
proxy::sse                  # SSE 解析与字段处理
    │
    ▼
stats::recorder             # 记录统计
    │
    ▼
返回客户端
```

## 代理核心模块说明

| 文件 | 职责 |
|------|------|
| `proxy/mod.rs` | 主入口，`handle_proxy_request` 统一处理所有代理请求 |
| `proxy/cache.rs` | `ProxyCache` 结构体，维护模型索引、渠道缓存 |
| `proxy/channel.rs` | 渠道选择逻辑 |
| `proxy/selection.rs` | 模型与分组选择策略 |
| `proxy/scheduler.rs` | API Key 轮询调度（`AtomicU64` 无锁 round-robin） |
| `proxy/circuit.rs` | 三态熔断器（关闭/打开/半开） |
| `proxy/prepare.rs` | 请求准备（认证 Header、上游 URL 组装） |
| `proxy/execute.rs` | 请求执行（流式/非流式，必要时重试） |
| `proxy/queue.rs` | 并发队列控制 |
| `proxy/state.rs` | 代理状态管理 |
| `proxy/sse.rs` | SSE 解析与上游错误清洗 |

## 协议转换层说明

| 文件 | 职责 |
|------|------|
| `protocol/inbound.rs` | 入站转换器 trait 定义 |
| `protocol/outbound.rs` | 出站转换器 trait 定义 |
| `protocol/openai_chat.rs` | OpenAI Chat Completions 协议处理 |
| `protocol/openai_responses.rs` | OpenAI Responses 协议处理 |
| `protocol/anthropic.rs` | Anthropic Messages 协议处理 |
| `protocol/model.rs` | 统一内部模型（`LlmRequest`, `LlmResponse`） |

**说明**: 当前协议转换实现直接位于 `protocol/*.rs` 文件中，未再拆分子目录。

## 同格式直通优化

当入站和出站协议相同时（如 Anthropic→Anthropic），可以跳过协议转换：

```
客户端请求
    │
    ▼
proxy::passthrough         # 检测到同格式
    │
    ▼
直接转发（仅替换 URL + 认证头）
    │
    ▼
上游响应（流式/非流式）
    │
    ▼
解析 SSE 事件提取 usage 信息（不解析业务字段）
    │
    ▼
记录统计 + 返回客户端
```

**统计记录**: 即使直通模式，也需要尽量保留上游响应中的 `usage` 信息用于统计。

## 流式转换缓冲策略

跨协议流式转换时，按目标协议的事件格式输出；实现上优先在可转换单元到达时立即转发，避免不必要的缓冲。

## 故障转移重试策略

| 场景 | 行为 |
|------|------|
| 非流式请求失败 | 继续走下一次渠道尝试 |
| 流式请求未开始返回 | 继续走下一次渠道尝试 |
| 流式请求已开始返回 | 向客户端返回错误事件，不重试 |
| 同渠道重试 | 由 `proxy::execute` 结合渠道状态控制 |
| 跨渠道切换 | 由 `ProxyState` 选择下一个可用渠道 |
