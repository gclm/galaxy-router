# 客户端配置生成器设计方案（v2 精简版）

> 状态：待实施 | 优先级：P0 | 预估：1-2 天（纯前端）
> 可交互草稿原型：`docs/client-config-prototype.html`（浏览器直接打开）
> 相对 v1 的核心变化见下方「设计共识」表

## 背景

用户在 galaxy-router 后台创建好 API Key 后，需要手动配置客户端（Claude Code / Codex）才能使用。当前「客户端配置」入口（`Sidebar.tsx`）只是一个外链到 GitHub README，用户拿到 key 不知道下一步怎么配。

**目标**：用户选好 API Key + 客户端类型，一键生成可直接粘贴/下载的配置，并告诉用户「粘到哪个文件、已有配置怎么合并」。

## 设计共识（v2 相对 v1 的关键变化）

| # | v1 草案 | v2 决策 | 理由 |
|---|---|---|---|
| base_url 来源 | 从 `config.toml` 读 host:port | **前端用 `window.location.origin`**，可手填覆盖 | config 里是监听地址（`0.0.0.0:8080`），不是客户端连接地址；用户浏览器地址大概率即连接地址 |
| 模型选择 | sonnet/opus/haiku 三槽位「模型映射」 | **保留三槽位**（仅 Claude Code），每个槽位下拉选 `group.name` | 三槽位对应 Claude Code 的档位默认模型；galaxy 的 `group.name` 即对外虚拟模型名，正好做下拉内容 |
| 生成方式 | 后端 `POST /client-config/generate` | **纯前端生成，无后端接口** | 配置本质是模板拼接；明文 key 前端已有（list 接口返回）；避免后端日志泄漏风险 |
| UI 布局 | 三栏（列表 \| 表单 \| JSON 编辑器） | **单栏流式 + 只读预览**（高级原始 JSON 折叠） | galaxy 用户 90% 是「建完 key 想马上能用」，不是管理多个渠道 |
| 触发时机 | 仅侧边栏独立页 | **创建 key 成功即弹** + 独立页保留 | 贴用户心智流：建完 key 那一刻最需要配置 |
| 安装引导 | 几乎没有 | **补「粘到哪个文件 / 怎么合并」** | 拿到配置不会用是真实断点 |

## 功能范围

### 支持的客户端

| 客户端 | 配置目标 | 模型配置 | 优先级 |
|---|---|---|---|
| Claude Code | `~/.claude/settings.json` | 三槽位（sonnet/opus/haiku） | P0 |
| Codex | `~/.codex/config.toml` + `~/.codex/auth.json` | 单默认模型 | P0 |
| Cursor / Cline | — | — | 不做 |

### 不做的事

- ❌ 直接「应用」配置到用户本机（服务端不可能）
- ❌ 后端生成接口（纯前端）
- ❌ 右侧「可编辑 JSON」（降为只读预览 + 折叠的原始 JSON）

## 核心设计：纯前端生成

**无后端接口**。配置在前端用模板拼接生成，数据全部来自已有接口：

| 数据 | 来源 | 已有？ |
|---|---|---|
| API Key 明文 | `GET /api/v1/admin/api-keys`（原样返回） | ✅ |
| 模型下拉选项 | `GET /api/v1/admin/groups`（`group.name` 即虚拟模型名） | ✅ |
| 网关 base_url | `window.location.origin` | ✅ 浏览器自带 |

生成逻辑放在 `frontend/src/lib/clientConfig.ts`，纯函数、可单测。

## UI 设计：单栏流式

```
┌──────────────────────────────────────────────┐
│  [ Claude Code ] [ Codex ]      ← 客户端 Tab  │
├──────────────────────────────────────────────┤
│  API Key   [ sk-gr-xxx          ▼ ]  ← 下拉   │
│  网关地址  [ https://router...     ]  ← 默认   │
│                                              │
│  ③ Claude Code: 三槽位                        │
│    Sonnet  [ claude-sonnet-4-6 ▼ ]           │
│    Opus    [ claude-opus-4-8   ▼ ]           │
│    Haiku   [ glm-4.6          ▼ ]            │
│  ③ Codex: 单模型                              │
│    默认模型 [ claude-sonnet-4-6 ▼ ]           │
│                                              │
│  选项  ☐ hideAttribution  ☐ effortMax  ☐ disableAutoUpdate │
├──────────────────────────────────────────────┤
│  生成的配置（只读预览）                        │
│  ┌────────────────────────────────────────┐ │
│  │ { "env": { "ANTHROPIC_BASE_URL": ... }}│ │
│  └────────────────────────────────────────┘ │
│  [ 复制配置 ]  [ 下载文件 ]  [ ▸ 原始 JSON ] │
├──────────────────────────────────────────────┤
│  ▸ 如何安装（粘到哪个文件 / 已有配置怎么合并） │
└──────────────────────────────────────────────┘
```

- 表单任意改动 → 预览实时刷新
- 「原始 JSON」默认折叠，给高级用户手动调整用
- 「如何安装」手风琴，按客户端类型给不同指引

## 触发时机

1. **创建 API Key 成功后**：成功提示里带「生成客户端配置」入口，跳转到 `/client-config` 且 key 已预填
2. **独立页 `/client-config`**：侧边栏「客户端配置」从外链 GitHub 改为内部路由，供回头查看

## 配置字段映射（⚠️ 实施前需核实最新客户端字段）

> 以下映射基于当前理解，**Claude Code / Codex 的配置字段更新较快，动手前必须核一遍官方文档**。带 ❓ 的为重点核实项。

### Claude Code → `~/.claude/settings.json`

```jsonc
{
  "env": {
    "ANTHROPIC_BASE_URL": "<网关地址>",
    "ANTHROPIC_API_KEY": "<api_key>",
    "ANTHROPIC_DEFAULT_SONNET_MODEL": "<sonnet 槽位的 group.name>",  // ❓ 待核实变量名
    "ANTHROPIC_DEFAULT_OPUS_MODEL":   "<opus 槽位的 group.name>",    // ❓ 待核实变量名
    "ANTHROPIC_DEFAULT_HAIKU_MODEL":  "<haiku 槽位的 group.name>"     // ❓ 待核实变量名
  }
}
```

开关（可选，按勾选追加）：
- `hideAttribution`（隐藏署名）→ `CLAUDE_CODE_HIDE_ATTRIBUTION: "1"`（❓ 待核实）
- `effortMax`（最高思考强度）→ ❓ 待核实对应字段
- `disableAutoUpdate`（禁用自动更新）→ ❓ 待核实对应字段

### Codex → `~/.codex/config.toml` + `~/.codex/auth.json`

```toml
# config.toml（❓ 整体结构待核实）
model_provider = "galaxy"
model = "<group.name>"

[model_providers.galaxy]
name = "Galaxy Router"
base_url = "<网关地址>/v1"
env_key = "GALAXY_API_KEY"
```

```jsonc
// auth.json（❓ 待核实）
{ "GALAXY_API_KEY": "<api_key>" }
```

## 安装引导内容（按客户端类型）

**Claude Code**
- 文件路径：`~/.claude/settings.json`
- 文件不存在 → 新建；已存在 → 把生成的 `env` 字段**合并**进现有 `env`（不要整体覆盖）
- 给一段合并示例

**Codex**
- `~/.codex/config.toml` + `~/.codex/auth.json` 两个文件
- 分别说明每个文件放什么

## 实施计划（纯前端，1-2 天）

1. `frontend/src/lib/clientConfig.ts`：`generateClaudeConfig` / `generateCodexConfig` 纯函数 + 单测（**先核实字段映射再写**）
2. `ClientConfigPage.tsx`：单栏 UI，按原型实现
3. 创建 API Key 成功回调里接入「生成客户端配置」入口（key 预填）
4. `Sidebar.tsx`：外链 GitHub → 内部路由 `/client-config`
5. 「如何安装」手风琴组件

## 风险与缓解

| 风险 | 缓解 |
|---|---|
| 客户端配置字段随版本变化 | 字段映射集中在 `clientConfig.ts` 一处，易改；预览 + 原始 JSON 让高级用户手动兜底 |
| `window.location.origin` 不准（反代/内网） | 字段可手填覆盖 |
| 明文 key 在前端 | 与现状一致（list 接口已返回明文），不引入新增风险；不持久化到 localStorage |

## 后续扩展（P2）

- 支持 Cursor / Cline
- 配置模板（预设常用组合）
- 多配置方案保存
