---
status: pending
---

# API Key 技术债清理 — 设计方案

## 目标
将 API Key 管理对齐 Channels/Groups 的成熟模式：
1. 后端加服务端分页
2. 前端抽出 `ApiKeyForm` 组件
3. 前端删除 client-side `useMemo`，改用 `useTableLoader` 服务端分页

## 执行顺序

### Phase 1: 后端 — 服务端分页

**Step 1.1: 添加 `ListApiKeysQuery` + 修改 `list` handler**
- 文件：`src/api/handlers/admin/api_keys.rs`
- 动作：
  - 新增 `ListApiKeysQuery` 结构体（`search/status/sort_by/sort_order/page/page_size`）
  - 修改 `list` 函数签名接收 `Query<ListApiKeysQuery>`
  - 添加 WHERE 条件（search 模糊匹配 name，status 过滤 enabled）
  - 添加 ORDER BY + LIMIT/OFFSET
  - 返回 `PaginatedResponse<ApiKey>`（复用 `channels/types.rs` 的 `PaginatedResponse`）
- 退出信号：`cargo test --lib` 通过
- 回滚策略：git revert

**Step 1.2: 更新测试**
- 文件：`tests/api/api_keys.rs`
- 动作：
  - `test_api_keys_list_empty` 和 `test_api_keys_list_with_data` 更新断言：`body["data"]["items"]` 替代 `body["data"]`，增加 `body["data"]["total"]` 断言
  - 新增 `test_api_keys_list_pagination`：创建 3 条，验证 page_size=2 返回 2 条 + total=3
  - 新增 `test_api_keys_list_search`：验证 search 过滤
- 退出信号：`cargo test` 全部通过
- 回滚策略：git revert

### Phase 2: 前端 API 层

**Step 2.1: 更新 `api-keys.ts`**
- 文件：`frontend/src/api/api-keys.ts`
- 动作：
  - 添加 `ApiKeyListParams` 接口（同 `ChannelListParams`）
  - `list` 方法接受 `params?` 参数，返回 `PaginatedResponse<ApiKey>`
- 退出信号：`pnpm build` 通过

### Phase 3: 前端 — 抽取 ApiKeyForm 组件

**Step 3.1: 新建 `ApiKeyForm.tsx`**
- 文件：`frontend/src/components/ApiKeyForm.tsx`（新建）
- 动作：
  - 接受 props：`apiKey?: ApiKey`, `groups: Group[]`, `budget?: BudgetLimit`, `onSubmit: (data) => void`, `onCancel: () => void`
  - 内含表单状态（name/rateLimit/groups/budget）+ GroupSelector + 提交逻辑
  - 创建/编辑共用，通过 `apiKey` 是否存在区分
  - budget 字段（月/日限额）仅在编辑模式显示
- 退出信号：`pnpm build` 通过

### Phase 4: 前端 — 重写 ApiKeys.tsx

**Step 4.1: 简化 ApiKeys.tsx**
- 文件：`frontend/src/pages/ApiKeys.tsx`
- 动作：
  - 删除 client-side `useMemo`（46-85 行）
  - `useTableLoader` 切换为服务端分页模式（传 params 给 `apiKeysApi.list`）
  - 替换内联表单为 `<ApiKeyForm>` 组件
  - 删除所有 create/edit 相关的 useState（约 10 个）
  - 保留：表格渲染 + Dialog 壳 + delete/toggle 确认 + copyToClipboard + newKeyResult
- 退出信号：`pnpm build` 通过 + HUMAN 确认功能正常

## 验收契约

| # | 场景 | 期望 |
|---|------|------|
| 1 | API Key 列表分页 | 翻页请求后端 |
| 2 | API Key 搜索 | 输入关键词过滤 |
| 3 | 创建 API Key | 表单弹出 → 填写 → 创建成功 |
| 4 | 编辑 API Key | 点击编辑 → 表单预填 → 保存成功 |
| 5 | 启用/禁用 | 确认框 → 操作成功 |
| 6 | 删除 | 确认框 → 删除成功 |
| 7 | Budget 编辑 | 编辑表单中修改月/日限额 |
| 8 | ApiKeys.tsx 行数 | < 300 行 |
