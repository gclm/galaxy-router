---
doc_type: refactor-report
feature: error-migration
status: done
---

# error-migration 验收报告

## 变更概要

将分散在 `relay/error.rs` 和 `api/response.rs` 的错误类型统一迁移到 `src/error/` 包。

## 变更文件

| 文件 | 动作 |
|------|------|
| `src/error/mod.rs` | 新建 — 模块入口 |
| `src/error/proxy.rs` | 新建 — ProxyError/ErrorClass/ErrorFormat + 分类/格式化 + 4 测试 |
| `src/error/app.rs` | 新建 — ApiError/ApiResponse/success_empty + 2 测试 |
| `src/relay/error.rs` | 删除 |
| `src/relay/mod.rs` | 移除 `pub mod error` |
| `src/api/response.rs` | 精简为只含 `generate_id()` + 1 测试 |
| `src/api/mod.rs` | 移除 `pub use response::{ApiError, ApiResponse}` |
| `src/main.rs` / `src/lib.rs` | 添加 `mod error` |
| `src/relay/pipeline.rs` | 更新内联路径引用 |
| `src/relay/{state,executor,candidates,prepare,stream_executor}.rs` | 更新 import |
| `src/api/handlers/proxy/{chat,messages,embeddings,images,responses}.rs` | 更新 import |
| `src/api/handlers/admin/*.rs`（9 个文件） | 更新 import + success_empty 路径 |
| `src/api/middleware/mod.rs` | 更新 import |
| `src/metrics/recorder/mod.rs` | 更新 import |
| `tests/integration_test.rs` | 更新测试路径引用 |
| `AGENTS.md` / `conventions.md` / `pitfalls.md` / `roadmap.md` | 同步知识层 |

## 验证结果

- `make check`：clippy 0 error + 全部测试通过
- 旧路径残留检查：无（grep 验证）
- 行为等价：无任何外部可观察行为变更

## 不在本次 scope

- 将 `use crate::error::proxy::*` 进一步简写（可选后续优化）
