---
doc_type: refactor-scan
feature: admin-restful
status: done
---

# admin-restful scan

## 总览

扫描范围：`src/api/router.rs`、`src/api/handlers/admin/`、`frontend/src/api/`、`tests/`
发现条数：5
建议先做：全部一次性完成
慎做：无

## 优化点清单

### 1. `/models/info` → `/models`，合并 `/fetch-models` → `/models/fetch`
- 分类：L1
- 位置：`src/api/router.rs`、`src/api/handlers/admin/model_info.rs`、`fetch_models.rs`
- 问题：`/models/info` 路径多余一层；`/fetch-models` 动词当资源名
- 建议：合并为 `/models` nest，需要统一 `ModelsState`（合并 `ModelInfoState` + `FetchModelsState`）
- 风险：中（状态类型合并）

### 2. `/backup/export|import|reset` → `/backups` GET/POST/DELETE
- 分类：L1
- 位置：`src/api/router.rs`
- 问题：动作当路径，非 RESTful
- 建议：`GET /backups` 导出、`POST /backups` 导入、`DELETE /backups` 重置
- 风险：低（纯路由注册变更，handler 不动）

### 3. `/stats/budgets` → `/budgets` 独立资源
- 分类：L1
- 位置：`src/api/router.rs`
- 问题：写操作混在只读 stats 下
- 建议：独立 `/budgets` nest，复用 `StatsApiState`
- 风险：低

### 4. 前端 API 路径同步
- 分类：L1
- 位置：`frontend/src/api/` 下 4 个文件
- 问题：前端路径需与后端同步
- 建议：一次性全改
- 风险：低

### 5. 测试路径同步
- 分类：L1
- 位置：`tests/` 下 4 个测试文件
- 问题：测试路径需与新路由对齐，`backup/reset` 方法从 POST 改为 DELETE
- 建议：一次性全改
- 风险：低
