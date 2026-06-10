# Project Basics

> 一屏读完，每次启动必读

## Tech Stack

- Rust 2024 (axum 0.8)
- SQLite (sqlx 0.9, `src/db/migrations/`)
- React + pnpm (frontend/)

## Commands

```bash
make build          # 构建项目（含前端 pnpm build）
make run            # 启动服务（等同于 cargo run）
make dev            # 前后端同时运行（cargo run + vite dev）
make test           # cargo test
make check          # clippy + test
make fmt            # cargo fmt
```

## Key Constraints

- 所有实体 ID 使用 UUID v7，TEXT 类型
- 管理 API (`/api/v1/admin/*`) 使用 `{code, message, data}` 格式；代理 API (`/v1/*`) 保持原生协议格式，不做包装
- 渠道 `models` 字段为 JSON 字符串数组 `["gpt-4o"]`，由 `parse_models` 解析
- 数据库迁移只增不改，版本号即文件名，`sqlx::migrate!()` 编译期管理
- 业务配置变更后必须同步失效 `ProxyCache`
- 错误类型在各模块内定义，不要统一到 `error.rs`
- `protocol/inbound/` 和 `protocol/outbound/` 已有内容，可以修改
- `relay/` = 代理请求生命周期，`scheduler/` = 负载均衡

## Architecture Summary

```
请求 → api/handlers/proxy/ → relay/pipeline → scheduler/selector → relay/executor → 上游
                                              ↓
                                        protocol/{inbound,outbound} 协议转换
```

## Config

- 配置文件：`config.toml`（TOML 格式）
- 环境变量覆盖前缀：`GALAXY_PROXY_`
- CLI 参数：`--port`、`--host`、`--config`、`--log-level`
