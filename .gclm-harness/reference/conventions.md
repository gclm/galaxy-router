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

- **业务错误**：各模块定义自己的 enum（`relay/error.rs::ProxyError`），用 `thiserror` 派生 `Error`
- **基础设施错误**：`anyhow::Result` 传播（DB 连接、配置加载）
- **HTTP 错误响应**：
  - 代理 API：按入站协议格式返回（OpenAI → `{"error": {...}}`，Anthropic → `{"type": "error", ...}`）
  - 管理 API：统一 `ApiError { code, message }` 格式
- **错误分类**：`ErrorClass` 枚举（KeyRetryable / UpstreamRetryable / Client / Internal）驱动重试策略

## 测试规范

- 单元测试写在同文件 `#[cfg(test)] mod tests { }` 内
- 测试函数命名：`snake_case_describes_expected_behavior`（如 `classify_upstream_distinguishes_key_retryable`）
- 测试数据库路径：`/tmp/galaxy_test_*`，每个测试模块用独立文件
- 集成测试依赖 `tower::ServiceExt` 对 Router 做 oneshot 请求
- 外部 HTTP 请求不 mock，测试只覆盖内部逻辑

## 代码风格

- Rust 2024 edition（let-chains / gen blocks 等新语法）
- `use` 语句按 crate 内 → 外部库 排序
- 中文字符串直接写在代码中（错误消息、日志），不做 i18n
- `tracing::info!/warn!/error!` 做日志，不用 `println!`
- `sqlx::query_as` / `query_scalar` 做数据库查询，类型安全
- 状态通过 axum `State` extractor 传递，不用全局 static
