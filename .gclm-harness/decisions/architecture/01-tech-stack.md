# 技术栈选型

## 核心技术栈

| 层级 | 技术 | 版本 | 理由 |
|------|------|------|------|
| 语言 | Rust | 2024 edition | 内存安全，零 GC，性能稳定 |
| HTTP 框架 | axum | 0.8 | 类型安全，tokio 生态 |
| 异步运行时 | tokio | 1.x | 生态最成熟 |
| HTTP 客户端 | reqwest | 0.13 | 流式支持好 |
| 数据库 | SQLite + sqlx | 0.9 | 轻量，迁移集成清晰 |
| 序列化 | serde + serde_json | 1.x | 零拷贝反序列化 |
| 配置 | config-rs + toml | 0.15 / 1.x | TOML 支持 |
| 日志 | tracing + tracing-subscriber | 0.1 / 0.3 | 结构化日志 |
| 流式处理 | tokio-stream / async-stream | 0.1 / 0.3 | 异步流抽象 |

## 可选依赖

| 功能 | 技术 | 说明 |
|------|------|------|
| 正则匹配 | regex | 模型名匹配 |
| 通配符匹配 | glob / 自实现 | 简单场景用自实现 |
| 嵌入静态资源 | rust-embed / include_dir | 管理前端随二进制分发 |
| 错误处理 | anyhow + thiserror | 应用层与模块内错误分工 |
| CLI 参数 | clap 4 | 命令行参数解析 |
| 认证 | argon2 + jsonwebtoken | 密码哈希与 JWT |

## 与 CCG Gateway 技术栈对比

| 维度 | CCG Gateway | Galaxy Router | 差异原因 |
|------|-------------|--------------|---------|
| HTTP 框架 | axum 0.7 | axum 0.8 | Galaxy Router 使用更新版 axum |
| 数据库 | SQLite + sqlx | SQLite + sqlx | 一致 |
| HTTP 客户端 | reqwest 0.12 | reqwest 0.13 | Galaxy Router 使用更新版 reqwest |
| 配置 | 环境变量 + DB | TOML 文件 + API | Galaxy Router 需要文件配置 |
| 桌面框架 | Tauri 2.0 | 无 | Galaxy Router 是服务端 |

## 与 AxonHub 技术栈对比

| 维度 | AxonHub (Go) | Galaxy Router (Rust) | Rust 等价物 |
|------|--------------|---------------------|-------------|
| Web 框架 | Gin | axum | — |
| ORM | Ent | sqlx | — |
| 依赖注入 | uber/fx | 无（手动组装） | Rust 通常不用 DI 框架 |
| 日志 | Zap | tracing | — |
| 流抽象 | 自定义 Stream[T] | futures::Stream | — |

## 编译目标

```toml
[profile.release]
opt-level = 3
lto = true
codegen-units = 1
strip = true
```

**目标二进制大小**: < 15MB（静态链接）
