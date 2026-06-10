---
doc_type: refactor-report
feature: admin-restful
status: done
---

# admin-restful 验收报告

## 变更概要

Admin API 路径对齐 RESTful 规范，前后端同步更新。

## 路由变更

| 旧路径 | 新路径 | 方法变更 |
|--------|--------|----------|
| `/models/info` GET/PUT | `/models` GET/PUT | - |
| `/models/info/{model}` GET | `/models/{model}` GET | - |
| `/fetch-models` POST | `/models/fetch` POST | - |
| `/backup/export` GET | `/backups` GET | - |
| `/backup/import` POST | `/backups` POST | - |
| `/backup/reset` POST | `/backups` DELETE | POST → DELETE |
| `/stats/budgets` GET | `/budgets` GET | - |
| `/stats/budgets` POST | `/budgets` POST | - |
| `/stats/budgets/{id}` DELETE | `/budgets/{id}` DELETE | - |

## 保持不变的路径

- `/channels/{id}/test` POST — 子资源操作，语义清晰
- `/channels/{id}/detect` POST — 同上

## 代码变更

| 文件 | 变更 |
|------|------|
| `src/api/handlers/admin/model_info.rs` | `ModelInfoState` → `ModelsState`（合并 fetch_client） |
| `src/api/handlers/admin/fetch_models.rs` | `FetchModelsState` → 复用 `ModelsState` |
| `src/api/router.rs` | 路由注册重构 |
| `frontend/src/api/{channels,model-info,backup,stats}.ts` | 路径同步 |
| `tests/api/{backup,models_info,stats}.rs` + `tests/e2e_integration.rs` | 测试路径同步 |

## 验证结果

- `make check`：clippy 0 error + 全部测试通过
- 知识层文档已同步

## 经验教训

- 任务文档应在开工前创建，不应跳过 SOP 步骤
