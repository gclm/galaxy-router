---
doc_type: refactor-scan
feature: error-migration
status: pending_review
---

# error-migration scan

## 总览

扫描范围：`src/relay/error.rs`、`src/api/response.rs` 及其所有消费者
发现条数：6
建议先做：全部（一次性搬移，有依赖关系）
慎做：无

## 迁移目标

将错误类型从分散位置迁入 `src/error/`：
- `ProxyError` + `ErrorClass` + `ErrorFormat` → `src/error/proxy.rs`
- `ApiError` + `ApiResponse` → `src/error/app.rs`（`generate_id()` 留在 `api/response.rs`）

不改名、不改行为、纯搬移。

## 优化点清单

### 1. 创建 `src/error/` 模块骨架
- 分类：L1（行为等价迁移）
- 位置：`src/error/mod.rs`（新建）
- 问题：当前无统一错误模块
- 建议：创建 `mod.rs`，声明 `pub mod app; pub mod proxy;`
- 风险：低

### 2. 搬迁 ProxyError 系列 → `src/error/proxy.rs`
- 分类：L1
- 位置：`src/relay/error.rs`（260 行）
- 问题：`ProxyError`、`ErrorClass`、`ErrorFormat`、`classify_upstream()`、`format_*` 系列函数、`render_status_and_message()` 全部定义在 relay/ 下，被 relay/、api/、metrics/ 三个模块跨域引用
- 建议：整文件搬入 `src/error/proxy.rs`，`relay/error.rs` 改为 `pub use crate::error::proxy::*;` 转发
- 风险：低（现有 `use crate::relay::error::X` 全部继续工作）
- 影响文件：12 处 import 路径（通过 re-export 无需改动）

### 3. 搬迁 ApiError + ApiResponse → `src/error/app.rs`
- 分类：L1
- 位置：`src/api/response.rs`（116 行）
- 问题：`ApiError`、`ApiResponse` 是管理 API 统一响应类型，放在 response.rs 中与 `generate_id()` 混杂
- 建议：`ApiError` + `ApiResponse` + 它们的 impl + 测试迁入 `src/error/app.rs`；`generate_id()` 留在 `api/response.rs`；`api/response.rs` 通过 re-export 转发
- 风险：低
- 影响文件：`api/mod.rs` 的 `pub use` 需更新

### 4. 注册 `error` 模块
- 分类：L1
- 位置：`src/main.rs`、`src/lib.rs`
- 问题：需要声明 `mod error;`
- 建议：在 `main.rs` 和 `lib.rs` 中添加 `mod error;`
- 风险：低

### 5. 更新 `api/mod.rs` 的 re-export
- 分类：L1
- 位置：`src/api/mod.rs:6`
- 问题：`pub use response::{ApiError, ApiResponse};` 需指向新位置
- 建议：改为 `pub use crate::error::app::{ApiError, ApiResponse};`（或保留 response re-export 二跳）
- 风险：低

### 6. 清理旧文件中已迁移的代码
- 分类：L1
- 位置：`src/relay/error.rs`、`src/api/response.rs`
- 问题：搬移后原文件只剩 re-export 转发 + `generate_id()`
- 建议：`relay/error.rs` → 纯 re-export；`api/response.rs` → `generate_id()` + re-export
- 风险：低

## 执行策略

采用 **Parallel Change**（Strangler Fig）：
1. 创建新位置，搬移代码
2. 旧位置改为 re-export（所有现有 import 继续工作）
3. 验证测试全绿
4. 可选后续：逐步将 `use crate::relay::error::X` 迁移为 `use crate::error::proxy::X`（不在本次 scope）

## 退出信号

- `cargo test` 全绿
- `cargo check` 无 warning
- 所有现有 import 路径仍然有效
