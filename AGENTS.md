# galaxy-router 开发约定

## 交互约定

- 遇到不确定的问题时，**必须停下来问用户，等回复后再继续**，不要自己假设答案
- 一次只问一个问题，不要一口气列 10 个问题
- 如果用户回复"你定"或"看着办"，才代表授权 AI 自行决策

## 约束（不能违反）

| # | 约束 | 原因 |
|---|------|------|
| 1 | 所有实体 ID 使用 UUID v7，TEXT 类型 | 全局唯一 + 时序可排序 |
| 2 | 管理 API (`/api/v1/admin/*`) 使用统一 JSON 格式：`{code, message, data}` | 与代理 API 分离，客户端 SDK 不受影响 |
| 3 | 代理 API (`/v1/*`) **保持原生协议格式**，不用统一响应包装 | 透传上游响应，确保 SDK 兼容 |
| 4 | 渠道 `models` 字段固定为 JSON 字符串数组：`["gpt-4o", "claude-3-5-sonnet"]` | 多处代码依赖此结构（`parse_models` / `Channel.models: Vec<String>`） |
| 5 | 数据库迁移：SQL 文件放在 `src/db/migrations/`，文件名 `{version}_{name}.sql` 且 version > 0；不可回滚、不可修改已发布版本 | 由 `sqlx::migrate!()` 在编译期管理，版本号即文件名 |
| 6 | 业务配置变更后，必须同步失效对应 `ProxyCache` | 缓存与数据库一致性 |

**踩坑警告**：
- 错误类型统一在 `src/error/` 包：`error::proxy`（ProxyError/ErrorClass/ErrorFormat）、`error::app`（ApiError/ApiResponse）；`generate_id()` 在 `api::response`
- `protocol/inbound/` 和 `protocol/outbound/` 已有协议转换实现（openai_chat/openai_responses/anthropic），可修改
- `relay/` 为代理请求生命周期模块，`scheduler/` 为负载均衡模块

## 快速参考

| 项 | 值 |
|----|----|
| 语言 | Rust 2024 |
| Web 框架 | axum 0.8 |
| 数据库 | SQLite (sqlx 0.9) |
| 异步运行时 | tokio 1.x |
| 前端 | React + pnpm |
| 测试数据库 | `/tmp/galaxy_test_*` |

| 命令 | 作用 |
|------|------|
| `make build` | 构建项目（含前端） |
| `make run` | 启动服务 |
| `make test` | 运行测试 |
| `make check` | 代码检查 |

## Git 提交

`feat` / `fix` / `refactor` / `test` / `docs` / `chore`: <description>
