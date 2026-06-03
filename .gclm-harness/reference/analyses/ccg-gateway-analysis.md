# CCG Gateway 项目分析

## 项目定位

为 Claude Code、Codex、Gemini CLI 打造的桌面端管理工具，集成了智能网关代理与配置管理功能。

## 技术栈

- 后端: Rust (axum + tokio + sqlx + reqwest)
- 前端: Vue 3 + TypeScript + Element Plus
- 桌面: Tauri 2.0
- 数据库: SQLite

## 我们借鉴什么

### 1. Rust 架构设计

```
api/handlers.rs      # HTTP 代理核心
services/proxy.rs    # 协议适配层
services/routing.rs  # 路由与故障转移
services/stats.rs    # 统计与日志
db/models.rs         # 数据模型
```

**借鉴点**: 分层清晰，职责分离明确。

### 2. 流式处理实现

- SSE chunk 级 token 解析
- 10MB 日志截断
- gzip/deflate/br/zstd 解压支持
- idle timeout 处理

**借鉴点**: 生产级流式处理代码，可直接参考。

### 3. CLI 类型检测

```rust
pub enum CliType { ClaudeCode, Codex, Gemini }

pub fn detect_cli_type(headers: &HeaderMap) -> CliType {
    let ua = headers.get("user-agent")...;
    if ua.contains("codex") || ua.contains("openai") { CliType::Codex }
    else if ua.contains("gemini") || ua.contains("google") { CliType::Gemini }
    else { CliType::ClaudeCode }
}
```

**借鉴点**: 简单有效的协议识别方式。

### 4. 故障拉黑机制

- 连续失败 N 次后自动拉黑 M 分钟
- `failure_threshold`: 默认 3 次
- `blacklist_minutes`: 默认 10 分钟
- 拉黑期间自动跳过，定期自动恢复

### 5. 统计体系

**三层统计**:
1. 实时统计 — 每次请求后写入 `usage_daily_model` 表
2. 请求日志 — 元数据 (SQLite) + 详情 (文件系统)
3. 历史回填 — 启动时批量回填

**统计维度**: 日期 + CLI 类型 + provider + 模型

### 6. 数据库 Schema 管理

- 三库分离 (主配置/日志/统计)
- Schema versioning + 自动迁移
- SchemaInspector / SchemaDiff / SchemaMigrator

## 关键差异

| 维度 | CCG Gateway | Galaxy Router |
|------|-------------|--------------|
| 协议转换 | 不做，透明代理 | 需要实现 |
| 配置方式 | DB + GUI | TOML 文件 |
| 部署方式 | 桌面应用 | 服务端 |
| 目标用户 | CLI 工具用户 | 通用 |

## 关键代码路径

| 模块 | 路径 |
|------|------|
| HTTP 代理核心 | `src-tauri/src/api/handlers.rs` |
| 协议适配层 | `src-tauri/src/services/proxy.rs` |
| 路由与故障转移 | `src-tauri/src/services/routing.rs` |
| 统计与日志 | `src-tauri/src/services/stats.rs` |
| 数据模型 | `src-tauri/src/db/models.rs` |
| Schema 定义 | `src-tauri/src/db/schema_definition.rs` |
