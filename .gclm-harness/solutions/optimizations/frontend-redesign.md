# Galaxy Router 前端面板重构方案

> 日期：2026-05-26
> 状态：待实施
> 范围：前端管理面板全面重构 + 后端 API 补充

---

## 1. 背景与目标

当前面板存在以下问题：

| 问题 | 现状 |
|------|------|
| 创建/编辑交互 | 整页替换为表单，丢失列表上下文 |
| API Key 创建 | 使用浏览器原生 `prompt()` 弹窗 |
| 设置页 | 仅有修改密码功能 |
| 视觉风格 | shadcn 默认色，缺少品牌感 |
| 渠道列表 | 无搜索、筛选、排序 |
| 分组编辑 | 无法从渠道快速添加模型 |
| 模型测试 | 内联面板，体验粗糙 |

**目标**：参考 nginxpulse 视觉风格 + octopus 交互模式 + sub2api 表格/筛选模式，全面重构。

**参考项目**：

| 项目 | 路径 | 参考点 |
|------|------|--------|
| nginxpulse | `/Users/gclm/workspace/refs/nginxpulse/` | 视觉风格、设置页 |
| octopus | `/Users/gclm/workspace/product/octopus/` | 渠道卡片、分组编辑器、测试 Dialog |
| sub2api | `/Users/gclm/workspace/lab/ai/sub2api/` | 渠道表格+筛选、测试终端 UI |

---

## 2. 技术决策

| 决策点 | 选择 | 理由 |
|--------|------|------|
| Dialog 组件 | 引入 shadcn/ui `dialog.tsx` | 项目已有 card/button，补一个即可 |
| 动效 | CSS transition + keyframe | 不引入 framer-motion，够用 |
| 虚拟滚动 | 暂不引入 | 渠道/分组数量级不大 |
| 筛选实现 | 前后端一起改 | 后端加查询参数，前端做搜索+筛选 UI |
| 分组模式切换 | 不需要 | galaxy-router 负载均衡只有一种模式 |
| 拖拽排序 | 暂不引入 | 用优先级输入框代替 |

---

## 3. Phase 1：基础组件补齐 & 视觉升级

### 3.1 引入 Dialog 组件

新增文件 `frontend/src/components/ui/dialog.tsx`。

使用 Radix UI Dialog 原语（shadcn/ui 标准方式），提供：

- `Dialog` / `DialogTrigger` / `DialogContent` / `DialogHeader` / `DialogTitle` / `DialogDescription` / `DialogFooter`
- 样式：`rounded-2xl`、`border`、`shadow-xl`、`backdrop-blur`
- 暗色模式适配

### 3.2 视觉升级

修改 `frontend/src/index.css`，参考 nginxpulse 的风格体系：

**主色调**（已完成部分）：
- 主色：`hsl(221 83% 53%)` — 蓝色系
- 圆角：`0.75rem` 基础，卡片 `1rem`

**新增样式**：

```css
/* body 背景：淡色径向渐变光晕 */
body {
  background-image:
    radial-gradient(1200px circle at 12% -18%, hsl(221 83% 53% / 0.06), transparent 58%),
    radial-gradient(900px circle at 90% -12%, hsl(40 96% 56% / 0.04), transparent 55%);
  background-attachment: fixed;
}

/* 卡片增强 */
.card-hover {
  hover:-translate-y-0.5 hover:shadow-lg transition-all duration-200
}

/* 主按钮渐变 */
.btn-primary {
  bg-gradient-to-r from-primary to-primary/90
  text-primary-foreground shadow-md shadow-primary/25
}
```

**输入框统一**：
- `rounded-xl`、`focus:ring-2 focus:ring-primary/30`

### 3.3 Sidebar 美化

修改 `frontend/src/components/layout/Sidebar.tsx`：

- Logo 区域：品牌名 + 渐变色块 icon
- 导航项：活跃项蓝色高亮 + 右侧小圆点指示器
- 底部添加设置入口（齿轮图标）
- 整体使用 `bg-sidebar` + `border-r` 样式

### 3.4 路由补充

修改 `frontend/src/App.tsx`，添加 `/settings` 路由。

---

## 4. Phase 2：渠道管理重构

### 4.1 后端 API 改造

修改 `src/api/handlers/admin/channels.rs` 的 `list` 函数：

**新增查询参数**：

| 参数 | 类型 | 说明 |
|------|------|------|
| `search` | `Option<String>` | 按名称模糊搜索 |
| `status` | `Option<String>` | 筛选 enabled/disabled |
| `sort_by` | `Option<String>` | 排序字段：name/created_at |
| `sort_order` | `Option<String>` | asc/desc |
| `page` | `Option<i32>` | 页码，默认 1 |
| `page_size` | `Option<i32>` | 每页数量，默认 20 |

**SQL 改造**：动态拼接 WHERE/ORDER BY/LIMIT 子句。

**响应格式**：

```json
{
  "items": [...],
  "total": 42
}
```

修改 `src/api/router.rs`，更新路由提取 query 参数。

### 4.2 前端 API 层

修改 `frontend/src/api/channels.ts`：

```ts
list: (params?: { search?: string; status?: string; sort_by?: string; sort_order?: string; page?: number; page_size?: number }) =>
  apiClient.get<{ items: Channel[]; total: number }>('/channels', { params }),
```

### 4.3 渠道列表改为表格 + 筛选

重写 `frontend/src/pages/Channels.tsx`。

**页面布局**：

```
┌─────────────────────────────────────────────────────┐
│  渠道管理                              [+ 创建渠道]  │
├─────────────────────────────────────────────────────┤
│  🔍 搜索渠道...   [全部▾] [启用▾]   🔄 刷新         │
├─────────────────────────────────────────────────────┤
│  名称 │ 端点    │ 状态 │ 模型数 │ Key数 │ 创建时间 │ 操作 │
│  ─────────────────────────────────────────────────  │
│  主力 │ OpenAI  │  ON  │  12   │  2   │ 05-20   │ ⋮  │
│  备用 │ Gemini  │ OFF  │   5   │  1   │ 05-18   │ ⋮  │
│  ...                                                 │
├─────────────────────────────────────────────────────┤
│  < 1  2  3 >                         共 42 条       │
└─────────────────────────────────────────────────────┘
```

**筛选栏**：
- 搜索框：按名称模糊搜索（前端 300ms debounce）
- 状态下拉：全部 / 启用 / 禁用
- 排序：点击表头排序（名称/创建时间）
- 刷新按钮
- 创建渠道按钮

**表格列**：

| 列 | 内容 | 交互 |
|----|------|------|
| 名称 | 加粗文本 | — |
| 端点 | 端点类型 badge | — |
| 状态 | Switch 开关 | 直接切换 enabled |
| 模型数 | 数字 | — |
| Key 数 | 数字 | — |
| 创建时间 | 格式化日期 | 可排序 |
| 操作 | 按钮 | 编辑 / 测试 / 删除 |

### 4.4 创建/编辑改为 Dialog

修改 `frontend/src/components/ChannelForm.tsx`，从整页替换改为 Dialog 内容。

- Dialog 宽度：`max-w-2xl`
- 内容可滚动（`max-h-[70vh] overflow-y-auto`）
- 沿用现有表单字段：名称、启用、API Keys、端点、模型配置、高级配置
- 点击表格行的编辑按钮或创建按钮打开

### 4.5 测试模型改为独立 Dialog（参考 sub2api）

重写测试模型交互，参考 sub2api 的 `AccountTestModal` 模式。

**Dialog 布局**：

```
┌──────────────────────────────────────┐
│  模型测试                      [✕]   │
├──────────────────────────────────────┤
│  ┌────────────────────────────────┐  │
│  │ ▶ OpenAI 主力渠道     ● 启用   │  │
│  └────────────────────────────────┘  │
│                                      │
│  选择模型：  [ gpt-4o ▾ ]           │
│                                      │
│  ┌────────────────────────────────┐  │
│  │  ▸ 正在连接 api.openai.com...   │  │
│  │  ✓ 连接成功                     │  │
│  │  → 发送测试: gpt-4o, max=5      │  │
│  │  ✓ 测试成功  耗时: 342ms        │  │
│  └────────────────────────────────┘  │
│                                      │
│                    [关闭] [开始测试]  │
└──────────────────────────────────────┘
```

**实现要点**：
- 顶部：渠道信息卡片（名称 + 状态 badge），渐变背景
- 模型选择：下拉框，自动选中第一个模型
- 输出区域：终端风格（`bg-gray-900 rounded-xl font-mono`）
  - 状态行：黄色 `正在连接...` → 绿色 `连接成功`
  - 结果行：绿色 `✓ 测试成功 耗时: Xms` / 红色 `✗ 测试失败: 原因`
- 按钮：关闭 + 开始测试/重试（颜色随状态变化）

---

## 5. Phase 3：分组管理重构

### 5.1 列表改为表格 + Dialog 编辑

重写 `frontend/src/pages/Groups.tsx`。

**表格列**：

| 列 | 内容 | 交互 |
|----|------|------|
| 名称 | 加粗 | — |
| 匹配规则 | code 样式 | — |
| 模型数 | badge 数字 | — |
| 渠道数 | 数字 | — |
| 重试 | 是/否 (最多 N 次) | — |
| 状态 | Switch | 直接切换 |
| 操作 | 按钮 | 编辑 / 删除 |

### 5.2 分组编辑 Dialog（参考 octopus 双面板）

修改 `frontend/src/components/GroupForm.tsx`。

**Dialog 宽度**：`max-w-4xl`

**表单区域（顶部）**：

| 字段 | 类型 |
|------|------|
| 分组名称 | 文本输入 |
| 匹配规则 | 文本输入（可选正则） |
| 重试开关 | Switch |
| 最大重试次数 | 数字输入（重试开启时显示） |
| 启用 | Switch |

**双面板区域（底部，占 Dialog 主体）**：

```
┌──────────────────────────┬──────────────────────────┐
│   从渠道添加模型          │   已选成员 (3)   [清空]   │
│ ┌──────────────────────┐ │ ┌──────────────────────┐ │
│ │ 🔍 搜索模型...        │ │ │ 1  gpt-4o            │ │
│ └──────────────────────┘ │ │   来源: OpenAI 主力   │ │
│                          │ │   优先级: [1]         │ │
│ ▼ OpenAI 主力渠道  3/5   │ ├──────────────────────┤ │
│   gpt-4o         [✓]    │ │ 2  claude-3.5         │ │
│   gpt-4o-mini    [+]    │ │   来源: Anthropic     │ │
│   gpt-4-turbo    [+]    │ │   优先级: [2]         │ │
│                          │ ├──────────────────────┤ │
│ ▼ Anthropic 渠道  0/2   │ │ 3  gemini-pro         │ │
│   claude-3.5     [+]    │ │   来源: Gemini        │ │
│   claude-3-haiku [+]    │ │   优先级: [3]         │ │
└──────────────────────────┴──────────────────────────┘
```

**左侧面板 — 模型选择器（ModelPickerSection）**：
- 按渠道分组（Accordion 折叠面板）
- 每个渠道显示：渠道名 + 已选/总数（如 `3/5`）
- 搜索框过滤模型名
- 已添加的模型显示 `✓` 灰化不可点击
- 未添加的模型显示 `[+]` 点击添加
- "一键添加"按钮：按分组名称自动匹配模型名

**右侧面板 — 已选成员（SelectedMembers）**：
- 列表显示：序号 + 模型名 + 来源渠道名 + 优先级输入框
- 每项有 `[✕]` 删除按钮
- 顶部有 "清空" 按钮

**数据来源**：
- 前端从 `GET /channels` 获取所有渠道及其 `models.available_models`
- 聚合为 `{ channelId, channelName, models: string[] }` 的列表
- 不需要额外后端 API

### 5.3 后端 API

分组相关的后端 API 已支持 CRUD，无需额外改造。

---

## 6. Phase 4：API Keys 页重写

### 6.1 创建 API Key 改为 Dialog

重写 `frontend/src/pages/ApiKeys.tsx`。

**创建流程**：

1. 点击 "创建 API Key" 按钮 → 打开 Dialog
2. Dialog 内：名称输入框 + 提交按钮
3. 创建成功后，**同一个 Dialog 内**展示：
   - 成功提示（绿色）
   - 完整 Key 文本（`font-mono`）+ 复制按钮
   - 警告提示："请立即复制保存，此密钥只会显示一次"
   - "我已保存" 按钮 → 关闭 Dialog

**列表改为表格**：

| 列 | 内容 | 交互 |
|----|------|------|
| 名称 | 文本 | — |
| Key | 脱敏显示 `sk-xxxx...xxxx` | 复制按钮 |
| 创建时间 | 格式化日期 | — |
| 状态 | Switch | 直接切换 |
| 操作 | 按钮 | 删除（确认弹窗） |

### 6.2 后端 API

已有 API 无需改造。创建接口已返回完整 key。

---

## 7. Phase 5：设置页补全

### 7.1 页面结构

重写 `frontend/src/pages/Settings.tsx`。

参考 nginxpulse 的设置页卡片风格，使用卡片分区布局：

```
┌─────────────────────────────────────────────────────┐
│  设置                                                │
├─────────────────────────────────────────────────────┤
│                                                      │
│  ┌─── 系统信息 ──────────────────────────────────┐  │
│  │  🖥  Galaxy Router v0.1.0                       │  │
│  │     运行时间: 3d 14h 22m                        │  │
│  │     Go 1.23.4 · SQLite 3.45.0                  │  │
│  └───────────────────────────────────────────────┘  │
│                                                      │
│  ┌─── 账户信息 ──────────────────────────────────┐  │
│  │  👤 admin                                      │  │
│  │     ID: a1b2c3d4                               │  │
│  └───────────────────────────────────────────────┘  │
│                                                      │
│  ┌─── 修改密码 ──────────────────────────────────┐  │
│  │  当前密码  [_____________]                      │  │
│  │  新密码    [_____________]                      │  │
│  │  确认密码  [_____________]                      │  │
│  │                              [修改密码]         │  │
│  └───────────────────────────────────────────────┘  │
│                                                      │
│  ┌─── 外观 ──────────────────────────────────────┐  │
│  │  主题                                          │  │
│  │  [☀ 亮色]  [🌙 暗色]  [💻 跟随系统]            │  │
│  └───────────────────────────────────────────────┘  │
│                                                      │
└─────────────────────────────────────────────────────┘
```

### 7.2 分区详情

| 分区 | 内容 | 后端需求 |
|------|------|---------|
| 系统信息 | 版本号、运行时间、Go 版本、SQLite 版本、渠道数、分组数、Key 数 | **新增** `GET /api/v1/admin/system-info` |
| 账户信息 | 用户名、ID（只读展示） | 已有 |
| 修改密码 | 当前密码 + 新密码 + 确认密码 | 已有 |
| 外观 | 亮/暗/系统三选一 pill 按钮，存储到 localStorage | 不需要后端 |

### 7.3 后端新增 API

新增 `src/api/handlers/admin/system_info.rs`：

```rust
// GET /api/v1/admin/system-info
{
  "version": "0.1.0",
  "uptime_secs": 123456,
  "go_version": "go1.23.4",
  "sqlite_version": "3.45.0",
  "channel_count": 5,
  "group_count": 3,
  "api_key_count": 8,
  "total_requests": 12345,
  "total_tokens": 678901
}
```

注册路由到 `src/api/router.rs`。

### 7.4 外观切换实现

- 使用 `localStorage` 存储主题偏好：`theme: light | dark | system`
- 切换时添加/移除 `dark` class 到 `<html>` 元素
- 遵循系统 `prefers-color-scheme` 媒体查询

---

## 8. Phase 6：Dashboard 美化

修改 `frontend/src/pages/Dashboard.tsx`。

**指标卡片增强**：

- 每个卡片左上角带图标方块（蓝色渐变背景）
- 数字用 `text-3xl font-bold`
- 底部加趋势指示（如果有昨日对比数据）

**布局**：

```
┌──────────┐  ┌──────────┐  ┌──────────┐
│ 📊 请求数 │  │ 💬 Token │  │ 💰 成本  │
│   12,345  │  │  678,901  │  │  $12.34  │
│           │  │           │  │          │
└──────────┘  └──────────┘  └──────────┘
```

---

## 9. 文件变更清单

| 文件 | Phase | 变更类型 | 说明 |
|------|-------|---------|------|
| `frontend/src/components/ui/dialog.tsx` | 1 | 新增 | shadcn Dialog 组件 |
| `frontend/src/index.css` | 1 | 修改 | 视觉升级（渐变背景、卡片增强） |
| `frontend/src/components/layout/Sidebar.tsx` | 1 | 修改 | 美化 + 添加设置入口 |
| `frontend/src/App.tsx` | 1 | 修改 | 添加 /settings 路由 |
| `src/api/handlers/admin/channels.rs` | 2 | 修改 | list 函数支持搜索/筛选/排序/分页 |
| `src/api/router.rs` | 2 | 修改 | 更新渠道路由提取 query 参数 |
| `frontend/src/api/channels.ts` | 2 | 修改 | list 方法支持查询参数 |
| `frontend/src/api/types.ts` | 2 | 修改 | 新增分页响应类型 |
| `frontend/src/pages/Channels.tsx` | 2 | 重写 | 表格+筛选+分页 |
| `frontend/src/components/ChannelForm.tsx` | 2 | 修改 | 适配 Dialog |
| `frontend/src/pages/Groups.tsx` | 3 | 重写 | 表格+Dialog 编辑 |
| `frontend/src/components/GroupForm.tsx` | 3 | 重写 | 双面板：模型选择器+已选成员 |
| `frontend/src/pages/ApiKeys.tsx` | 4 | 重写 | Dialog 创建+表格列表 |
| `frontend/src/api/types.ts` | 4 | 修改 | 添加设置页相关类型 |
| `src/api/handlers/admin/system_info.rs` | 5 | 新增 | 系统信息 API |
| `src/api/handlers/admin/mod.rs` | 5 | 修改 | 注册 system_info 模块 |
| `src/api/router.rs` | 5 | 修改 | 注册 system_info 路由 |
| `frontend/src/pages/Settings.tsx` | 5 | 重写 | 多分区设置页 |
| `frontend/src/pages/Dashboard.tsx` | 6 | 修改 | 指标卡片美化 |

---

## 10. 执行顺序

```
Phase 1 → Phase 2 → Phase 3 → Phase 4 → Phase 5 → Phase 6
基础+视觉   渠道重构   分组重构   API Keys  设置页   Dashboard
```

每个 Phase 完成后：
- 启动 `cargo run` + `cd frontend && pnpm dev` 验证
- 检查亮/暗模式兼容性
- 检查移动端响应式（sm/md/lg 断点）
- 提交代码

---

## 11. 风险与注意事项

| 风险 | 缓解措施 |
|------|---------|
| Dialog 组件引入增加包体积 | Radix UI Dialog gzip 后约 5KB，可接受 |
| 后端 channels list SQL 拼接需防注入 | 使用参数化查询，不拼接用户输入到 SQL 字符串 |
| 双面板在移动端显示不佳 | md 以上断点用双面板，sm 用单面板 Tab 切换 |
| 分页状态丢失 | 筛选/排序变更时重置到第 1 页 |
| 外观切换闪烁 | 在 `<head>` 中加 blocking script 读取 localStorage 设置主题 |
