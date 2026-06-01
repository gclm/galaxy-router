# Galaxy Router 开发规范

## 项目概述

AI 协议互转代理网关，支持 OpenAI Chat Completions、OpenAI Responses、Anthropic Messages 三种协议互转。

## 技术栈

- 语言: Rust 2024
- Web 框架: axum 0.8
- 数据库: SQLite (sqlx 0.9)
- 异步运行时: tokio 1.x

## API 设计规范

### 响应格式分层

**管理 API** (`/api/v1/admin/*`): 统一 JSON 格式

```json
{ "code": 0, "message": "success", "data": { ... } }   // 成功
{ "code": 0, "message": "success", "data": null }       // 成功（无数据）
{ "code": 400, "message": "错误描述" }                   // 错误
```

**代理 API** (`/v1/*`): 保持原生协议格式，直接透传上游响应，确保客户端 SDK 兼容。

### ID 规范

- UUID v7，TEXT 类型，格式: `xxxxxxxx-xxxx-7xxx-xxxx-xxxxxxxxxxxx`

### 错误处理

```rust
ApiError::bad_request("参数错误")      // 400
ApiError::unauthorized("未授权")       // 401
ApiError::forbidden("禁止访问")        // 403
ApiError::not_found("资源不存在")      // 404
ApiError::conflict("资源冲突")         // 409
ApiError::internal_error("服务器错误") // 500
```

### 认证架构

| 层级 | 机制 | 保护范围 |
|------|------|----------|
| `require_admin_auth` 中间件 | JWT Bearer Token | `/api/v1/admin/*`（排除 init、login） |
| `ApiKeyAuth` 提取器 | API Key（Bearer 或 x-api-key） | `/v1/*` 代理端点 |

JWT 过期时间从 `config.toml` 的 `auth.token_expiry_hours` 读取，默认 24 小时。

## 数据库规范

- 所有表 ID 为 TEXT (UUID v7)，时间字段 TIMESTAMP 默认 CURRENT_TIMESTAMP
- 迁移使用内置系统，版本号递增不可回滚，追加到 `get_migrations()`

## 代码规范

### 模块职责

| 模块 | 职责 |
|------|------|
| `api/` | HTTP 路由、请求处理 |
| `api/handlers/admin/` | 管理 API 处理器 |
| `api/handlers/proxy/` | 代理 API 处理器（统一走 `handle_proxy_request`） |
| `auth/` | 密码哈希、JWT |
| `proxy/` | 代理核心（缓存、模型索引、渠道选择、协议转换） |
| `stats/` | 统计聚合（用量/成本）+ 模型定价 + 请求日志 |

### 命名规范

- 文件名/函数/变量: snake_case，结构体: PascalCase，常量: SCREAMING_SNAKE_CASE

### 测试

- 单元测试: `#[test]` / `#[tokio::test]`
- 集成测试: `tests/integration_test.rs`
- 测试数据库: `/tmp/galaxy_test_*`

## 架构要点

- **代理统一入口**: 所有代理 handler 统一调用 `handle_proxy_request`
- **缓存共享**: `ProxyCache` 被 admin handler 和 proxy 层共享，写操作后自动失效
- **模型反向索引**: `ProxyCache.model_index` 维护 model→channel_id 映射
- **API Key 轮询**: `AtomicU64` 无锁 round-robin；key 级 401/402/429 在同渠道内重试其他 key；渠道级失败排除整个 channel
- **上游错误脱敏**: `sanitize_upstream_error` 截断并提取关键信息
- **模型访问控制**: API Key 的 `supported_models` 字段控制模型范围，空=允许所有
- **SSE 格式兼容**: `sse_field()` 统一处理 `field:value` 和 `field: value` 两种格式
- **Token 兜底**: 上游不返回 usage 时，`estimate_tokens()` 从内容长度估算（~3 字节/token）

## Git 规范

```
<type>: <description>
```

Type: `feat` / `fix` / `refactor` / `test` / `docs` / `chore`

## 排查 SOP

- [Homebrew 部署与排查](.gclm-harness/sop/homebrew-deploy.md) — 服务路径、日志时间对齐、流式错误记录规则、部署流程
- [Token 统计缺失排查](.gclm-harness/sop/token-stats-debug.md) — 诊断 SQL、上游 curl 验证、提取函数定位、修复策略
