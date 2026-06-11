---
doc_type: refactor-scan
feature: frontend-optimization
status: reviewed
---

# frontend-optimization scan

## 总览

扫描范围：`frontend/src/` 全部页面 + API 层 + 通用组件
发现条数：15
建议先做：全部（按 Phase 顺序执行）
慎做：#4（服务端分页需要后端 API Keys 分页支持）

## 优化点清单

### Phase 1: P0 功能缺陷（3 条）

#### 1. 统计页面刷新按钮无效
- 分类：L2
- 位置：`ModelStats.tsx:99`, `ChannelStats.tsx:109`, `ApiKeyStats.tsx:71`
- 问题：`onRefresh={() => {}}` — 刷新按钮点击无效果
- 建议：从 query 解构 `refetch`，传入 `onRefresh`
- 风险：低

#### 2. Logs 刷新用 hack 而非 React Query
- 分类：L2
- 位置：`Logs.tsx:122-124`
- 问题：`window.dispatchEvent(new Event('focus'))` 触发 refetch
- 建议：直接调用 `refetch()`
- 风险：低

#### 3. Logs 页面缺少 title
- 分类：L2
- 位置：`Logs.tsx:128`
- 问题：`<PageHeader subtitle="..." />` 无 title prop
- 建议：加 `title="请求日志"`
- 风险：低

### Phase 2: P1 组件拆分 & 一致性（3 条）

#### 4. 服务端分页：Channels / Groups 拉全量数据
- 分类：L4
- 位置：`Channels.tsx:40-44`, `Groups.tsx:42-48`
- 问题：`list()` 不传分页参数，全量加载后客户端分页。API 已支持 `page/page_size`
- 建议：传分页参数到后端，移除客户端分页逻辑
- 风险：中（需验证后端分页行为与客户端一致）
- 注意：`apiKeysApi.list()` 后端不支持分页（返回 `ApiKey[]` 非 `PaginatedResponse`），本次不改

#### 5. ApiKeys 页面 689 行巨型组件
- 分类：L3
- 位置：`ApiKeys.tsx` 全文
- 问题：表格 + 创建表单 + 编辑表单 + GroupSelector 全部内联，15 个 useState
- 建议：抽成 `ApiKeyForm` 组件 + `GroupSelector` 独立组件，与 ChannelForm/GroupForm 风格对齐
- 风险：中（大量代码移动，需确保行为等价）

#### 6. Groups 页面用 raw promise 而非 React Query
- 分类：L2
- 位置：`Groups.tsx:37-39`
- 问题：`channelsApi.list().then(...).catch(console.error)` 无 loading/error 状态
- 建议：改用 `useChannels()` hook
- 风险：低

### Phase 3: P2 用户体验（6 条）

#### 7. 添加 React Error Boundary
- 分类：L3
- 位置：`App.tsx`
- 问题：组件渲染异常 = 白屏
- 建议：在 App 层添加 ErrorBoundary 组件，显示友好错误页
- 风险：低

#### 8. 添加 404 页面
- 分类：L3
- 位置：`App.tsx`
- 问题：未知路由无 Not Found 提示
- 建议：添加 `<Route path="*">` 兜底路由 + NotFound 页面
- 风险：低

#### 9. 启用/禁用操作增加确认
- 分类：L2
- 位置：`Channels.tsx:137-150`, `Groups.tsx:137-150`, `ApiKeys.tsx:233-246`
- 问题：点击 StatusBadge 直接切换，无二次确认
- 建议：复用已有的 `ConfirmDeleteDialog` 模式，加 toggle 确认弹窗
- 风险：低

#### 10. 排序箭头区分升序/降序
- 分类：L2
- 位置：`Channels.tsx:225`, `Groups.tsx:225` 等
- 问题：活跃排序列只显示 `ArrowUpDown`，不区分方向
- 建议：活跃升序用 `ArrowUp`，活跃降序用 `ArrowDown`，非活跃不显示图标
- 风险：低

#### 11. Dark mode 色值不一致
- 分类：L2
- 位置：`ApiKeys.tsx:497`, `Logs.tsx:222-233` 等
- 问题：部分硬编码色值缺少 `dark:` 变体
- 建议：统一补充 `dark:` 前缀或改用 CSS 变量
- 风险：低

#### 12. 添加全局页面切换 loading 指示器
- 分类：L3
- 位置：全局
- 问题：路由切换时无过渡反馈
- 建议：顶部进度条（nprogress 或自定义）
- 风险：低
- ~~建议：暂不做，当前页面切换足够快，加 nprogress 需要额外依赖。如需要后续单独加~~ → 用户要求全部修复，保留

### Phase 4: P3 代码质量（3 条）

#### 13. Dashboard 变量命名混淆
- 分类：L2
- 位置：`Dashboard.tsx:83-87`
- 问题：`sortedDaily`（排序后）和 `daily`（原始）命名不直观
- 建议：重命名为 `chartData`（排序后给图表）和 `rawDaily`（原始给汇总）
- 风险：低

#### 14. Playground 中 apiKeysApi 用 raw promise
- 分类：L2
- 位置：`Playground.tsx:194`
- 问题：`apiKeysApi.list().then(...)` 无 loading 状态
- 建议：改用 `useApiKeys()` hook + `useEffect` 初始化
- 风险：低

#### 15. 统计页面 FilterBar 简化版与完整版不统一
- 分类：L2
- 位置：`ModelStats.tsx`, `ChannelStats.tsx`, `ApiKeyStats.tsx`
- 问题：传 `onRefresh={() => {}}` 渲染了无效按钮（已在 #1 中修复）
- 建议：与 #1 合并处理
- 风险：低
