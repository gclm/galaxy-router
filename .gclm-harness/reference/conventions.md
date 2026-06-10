# 编码规范

> 从现有代码提取的命名和风格约定

## 命名约定

- **模块/文件**: snake_case (`relay/`, `stream_executor.rs`)
- **Struct**: PascalCase (`ProxyState`, `ChannelInfo`, `RelayRun`)
- **Enum**: PascalCase 变体 (`ProxyError::NoAvailableChannel`, `ErrorClass::KeyRetryable`)
- **函数/方法**: snake_case (`build_relay_candidates`, `handle_proxy_request`)
- **常量**: UPPER_SNAKE_CASE (`CACHE_TTL_SECS`, `KEY_NEEDLES`)
- **数据库表**: snake_case (`usage_logs`, `api_keys`, `group_items`)
- **API 路径**: kebab-case (`/api-keys`, `/api/v1/admin`)

## 错误处理

- **业务错误**：统一在 `src/error/` 包中定义
  - `error::proxy` — `ProxyError`（代理层错误枚举）、`ErrorClass`（重试策略分类）、`ErrorFormat`（响应格式）
  - `error::app` — `ApiError`（管理后台错误响应）、`ApiResponse`（管理后台成功响应）
- **基础设施错误**：`anyhow::Result` 传播（DB 连接、配置加载）
- **HTTP 错误响应**：
  - 代理 API：按入站协议格式返回（OpenAI → `{"error": {...}}`，Anthropic → `{"type": "error", ...}`）
  - 管理 API：统一 `ApiError { code, message }` 格式
- **错误分类**：`ErrorClass` 枚举（KeyRetryable / UpstreamRetryable / Client / Internal）驱动重试策略
- **工具函数**：`generate_id()` 在 `api::response` 模块（UUID v7 生成）

## 测试规范

- 单元测试写在同文件 `#[cfg(test)] mod tests { }` 内
- 测试函数命名：`snake_case_describes_expected_behavior`（如 `classify_upstream_distinguishes_key_retryable`）
- 测试数据库路径：`/tmp/galaxy_test_*`，每个 `TestApp::new()` 创建独立 SQLite 文件
- 集成测试使用 `TestApp`（`tests/common/app.rs`）构建 Router + DB + JWT + fixtures
- 代理测试使用 `wiremock` 模拟上游（`tests/common/mock.rs`），覆盖完整转发链路
- 外部 HTTP 请求全部 mock，不依赖真实网络
- 分类执行：`make test-api` / `make test-proxy` / `make test-regression`

## 代码风格

- Rust 2024 edition（let-chains / gen blocks 等新语法）
- `use` 语句按 crate 内 → 外部库 排序
- 中文字符串直接写在代码中（错误消息、日志），不做 i18n
- `tracing::info!/warn!/error!` 做日志，不用 `println!`
- `sqlx::query_as` / `query_scalar` 做数据库查询，类型安全
- 状态通过 axum `State` extractor 传递，不用全局 static
