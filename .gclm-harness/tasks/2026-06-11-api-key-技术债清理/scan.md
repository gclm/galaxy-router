---
doc_type: refactor-scan
feature: api-key-tech-debt-cleanup
status: reviewed
---

# API Key 技术债清理 scan

## 总览
扫描范围：API Key 管理全链路（后端 handler + 前端页面 + 前端 API 层 + 测试）
发现条数：4
建议先做：1、2（核心价值）
慎做：无

## 优化点清单

### 1. 后端 API Key list 缺少服务端分页
- 分类：L3
- 位置：`src/api/handlers/admin/api_keys.rs:73-114`
- 问题：`list` 返回 `Vec<ApiKey>` 无分页/搜索/排序，Channels/Groups 均已支持 `ListQuery` + `PaginatedResponse`
- 建议：参照 `channels/crud.rs` 的 `ListChannelsQuery` + `PaginatedResponse` 模式，给 API Key 加上 `page/page_size/search/status/sort_by/sort_order` 参数
- 风险：低（admin 内部 API，前后端同步改）

### 2. ApiKeys.tsx 表单逻辑内联 ~690 行
- 分类：L3
- 位置：`frontend/src/pages/ApiKeys.tsx`
- 问题：ChannelForm/GroupForm 已抽为独立组件，ApiKeys 仍全量内联。创建/编辑表单 + GroupSelector + budget 处理混杂在页面组件中
- 建议：抽出 `ApiKeyForm` 组件（含 GroupSelector + budget 字段），ApiKeys.tsx 仅保留表格 + Dialog 壳
- 风险：低（纯结构重组，行为不变）

### 3. 前端 client-side useMemo 可删除（依赖 #1）
- 分类：L2
- 位置：`frontend/src/pages/ApiKeys.tsx:46-85`
- 问题：因后端无分页，前端用 `useMemo` 做全套 filter/sort/paginate。后端加分页后可直接删除
- 建议：改用 `useTableLoader` 服务端分页模式（同 Channels/Groups）
- 风险：低

### 4. 前端 API 层需同步更新
- 分类：L1
- 位置：`frontend/src/api/api-keys.ts`
- 问题：`list()` 签名需接受查询参数，返回 `PaginatedResponse<ApiKey>`
- 建议：参照 `channels.ts` 添加 `ApiKeyListParams`，更新 `list` 方法签名
- 风险：低

## 前置检查

- ✅ 测试覆盖：13 个 API Key 测试（CRUD + disabled proxy）
- ✅ 范围：~6 文件，远低于 15 文件上限
- ✅ 行为等价：admin 内部 API，前后端同步改，用户可见行为不变
