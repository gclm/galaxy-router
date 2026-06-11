---
doc_type: refactor-design
status: approved
feature: 2026-06-10-前端全面优化修复缺陷提升可扩展性与用户体验
summary: 前端全面优化：修复缺陷、抽取通用组件、提升可扩展性与用户体验
---

# 前端全面优化：修复缺陷、抽取通用组件、提升可扩展性与用户体验

## 1. 决策与约束

### 用户目标
1. 修复审计发现的 15 个前端问题
2. 抽取通用组件（SortHeader / DataTable），减少 7+ 处重复代码
3. 增强 `useTableLoader` hook，统一数据加载模式

### 核心行为
- 所有页面功能行为等价（启用/禁用新增确认弹窗为明确需求变更）
- 不引入新 npm 依赖（进度条用纯 CSS 实现）
- 不改后端 API（API Keys 分页暂不做）

### 成功标准
1. `pnpm build` 无报错
2. 所有现有页面功能正常（HUMAN 验证）
3. 15 条审计问题全部修复
4. `SortHeader` 替换 4+ 处重复排序逻辑
5. `DataTable` 替换 7 处表格外壳重复代码
6. `useTableLoader` 支持 clientSide 模式，3 个页面删除重复 useMemo

### 明确不做
- API Keys 服务端分页（后端返回 `ApiKey[]` 非 `PaginatedResponse`，需后端先改）
- 引入 nprogress 等新 npm 依赖
- CSS 变量体系重构（仅补 dark mode 缺失值）
- 通用 CRUD 页面组件（列定义差异太大）
- 通用 Form 组件（各表单差异大，不值得统一）

## 2. 名词层 + 编排层

### 新增文件

| 文件 | 用途 | 减少重复 |
|------|------|---------|
| `components/common/SortHeader.tsx` | 排序表头组件 | 4 处 → 1 组件 |
| `components/common/DataTable.tsx` | 表格外壳 + 分页 + 空状态 | 7 处 → 1 组件 |
| `components/ErrorBoundary.tsx` | React 错误边界 | 防白屏 |
| `pages/NotFound.tsx` | 404 页面 | 路由兜底 |

### 变更文件

| 文件 | 变化 |
|------|------|
| `hooks/useTableLoader.ts` | 新增 `clientSide` 模式，内置 filter/sort/paginate |
| `components/common/index.ts` | 导出 SortHeader、DataTable |
| `pages/index.ts` | 导出 NotFound |
| `App.tsx` | ErrorBoundary 包裹 + NotFound 路由 + 全局进度条 |
| `pages/Channels.tsx` | 服务端分页 + SortHeader + DataTable + 启用/禁用确认 |
| `pages/Groups.tsx` | useChannels hook + 服务端分页 + SortHeader + DataTable + 启用/禁用确认 |
| `pages/ApiKeys.tsx` | 就地整理 + SortHeader + DataTable + 启用/禁用确认 |
| `pages/Logs.tsx` | 修 title + 修 refresh + DataTable |
| `pages/ModelStats.tsx` | 修 refresh + SortHeader + DataTable |
| `pages/ChannelStats.tsx` | 修 refresh + SortHeader + DataTable |
| `pages/ApiKeyStats.tsx` | 修 refresh + DataTable |
| `pages/Dashboard.tsx` | 变量重命名 |
| `pages/Playground.tsx` | useApiKeys hook |

### 现状 → 变化

**SortHeader**：
- 现状：4 处各自写 `<button onClick={sort}><label/><ArrowUpDown/></button>`
- 变化：`<SortHeader label="名称" field="name" sortBy={sortBy} sortOrder={sortOrder} onSort={handleSort} />`
  - 活跃列显示 ArrowUp / ArrowDown（区分方向）
  - 非活跃列无箭头

**DataTable**：
- 现状：7 处重复写 `<div className="rounded-2xl border bg-card...">` + thead + EmptyState + Pagination
- 变化：`<DataTable columns={[...]} loading={...} empty={...} pagination={...}>rows</DataTable>`
  - 统一表格外壳样式
  - 内置 EmptyState（loading/empty 态自动处理）
  - 内置 Pagination

**useTableLoader clientSide 模式**：
- 现状：Channels/Groups/ApiKeys 各写 ~30 行 useMemo 做 filter/sort/paginate
- 变化：`useTableLoader({ fetchFn, clientSide: true })` → hook 内部处理
  - ApiKeys 使用 clientSide 模式（后端无分页）
  - Channels/Groups 使用服务端分页（传参给后端）

**启用/禁用确认**：
- 现状：点击 StatusBadge 直接调 API
- 变化：复用 `ConfirmDeleteDialog`，文案改为"确定要启用/禁用此 {类型} 吗？"

## 3. 验收契约

| # | 场景 | 期望 |
|---|------|------|
| 1 | 统计页面刷新 | 点击刷新按钮，数据重新加载，按钮有 loading 态 |
| 2 | Logs 刷新 | 点击刷新按钮，数据重新加载 |
| 3 | Logs 标题 | 页面标题显示"请求日志" |
| 4 | Channels 分页 | 翻页请求后端，URL 带分页参数 |
| 5 | Groups 分页 | 同上 |
| 6 | ApiKeys 功能 | 创建/编辑/删除/启用/禁用 API Key 功能与原来完全一致 |
| 7 | Groups channels | channels 通过 React Query 加载（无 raw promise） |
| 8 | 渲染异常 | 组件 throw Error 时显示错误边界页面，非白屏 |
| 9 | 404 路由 | 访问 /foobar 显示 404 页面 |
| 10 | 启用/禁用确认 | 点击状态开关弹出确认框，确认后才执行 |
| 11 | 排序箭头 | 活跃列显示 ↑ 或 ↓，非活跃列无箭头 |
| 12 | Dark mode | 所有色值正常，无硬编码亮色块 |
| 13 | 页面切换 | 顶部出现进度条动画 |
| 14 | Dashboard 命名 | 变量名为 `chartData` / `rawDaily` |
| 15 | Playground | API Key 列表正常加载 |
| 16 | DataTable 统一 | 7 个页面的表格使用同一 DataTable 组件 |
| 17 | SortHeader 统一 | 4+ 处排序表头使用同一 SortHeader 组件 |

## 4. 推进策略

### Phase 1: P0 Bug 修复（3 步）

退出信号：3 个 bug 修复，`pnpm build` 通过

| Step | 动作 | 文件 |
|------|------|------|
| 1.1 | 修复统计页面刷新按钮：从 query 解构 `refetch`，传入 `onRefresh` | ModelStats, ChannelStats, ApiKeyStats |
| 1.2 | 修复 Logs 刷新：改用 `refetch()`，移除 focus hack | Logs.tsx |
| 1.3 | 修复 Logs 标题：加 `title="请求日志"` | Logs.tsx |

### Phase 2: 通用组件抽取（3 步）

退出信号：组件可用，`pnpm build` 通过

| Step | 动作 | 文件 |
|------|------|------|
| 2.1 | 新建 SortHeader 组件 | components/common/SortHeader.tsx |
| 2.2 | 新建 DataTable 组件（含 EmptyState + Pagination） | components/common/DataTable.tsx |
| 2.3 | 增强 useTableLoader：新增 clientSide 模式 | hooks/useTableLoader.ts |

### Phase 3: 页面改造 — 应用通用组件（4 步）

退出信号：所有 CRUD 列表页使用 DataTable + SortHeader，行为等价

| Step | 动作 | 文件 |
|------|------|------|
| 3.1 | Channels 改造：服务端分页 + DataTable + SortHeader | Channels.tsx |
| 3.2 | Groups 改造：useChannels hook + 服务端分页 + DataTable + SortHeader | Groups.tsx |
| 3.3 | ApiKeys 改造：clientSide 模式 + DataTable + SortHeader + 就地整理 | ApiKeys.tsx |
| 3.4 | Logs 改造：DataTable 替换表格外壳 | Logs.tsx |

### Phase 4: 基础设施 + 交互优化（5 步）

退出信号：ErrorBoundary + 404 + 确认弹窗 + 排序图标全部落地

| Step | 动作 | 文件 |
|------|------|------|
| 4.1 | 新建 ErrorBoundary，包裹 App | ErrorBoundary.tsx, App.tsx |
| 4.2 | 新建 NotFound 页面，添加兜底路由 | NotFound.tsx, App.tsx, pages/index.ts |
| 4.3 | 启用/禁用加确认（复用 ConfirmDeleteDialog） | Channels, Groups, ApiKeys |
| 4.4 | 统计页改造：SortHeader + DataTable + 修 refresh | ModelStats, ChannelStats, ApiKeyStats |
| 4.5 | 添加页面切换进度条（纯 CSS） | App.tsx |

### Phase 5: 收尾清理（3 步）

退出信号：全部优化完成，`pnpm build` 通过

| Step | 动作 | 文件 |
|------|------|------|
| 5.1 | Dashboard 变量重命名 | Dashboard.tsx |
| 5.2 | Playground 改用 useApiKeys hook | Playground.tsx |
| 5.3 | 补全 dark mode 缺失色值 | ApiKeys.tsx, Logs.tsx 等 |

## 5. 挂载点清单

- `App.tsx` — ErrorBoundary 包裹位置 + NotFound 路由注册 + 进度条
- `pages/index.ts` — 导出 NotFound
- `components/common/index.ts` — 导出 SortHeader、DataTable
- `hooks/useTableLoader.ts` — clientSide 模式新增
