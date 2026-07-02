---
doc_type: design
feature: 2026-07-02-客户端配置生成器
status: approved
complexity: moderate
---

# 客户端配置生成器

> 纯前端功能。用户建好 API Key 后，选客户端类型 → 一键生成可粘贴/下载的配置 + 安装指引。
> 替代当前「客户端配置」侧边栏只外链 GitHub README 的断点体验。
> 字段映射已抓官方文档核实（见 §2.4 出处）。原型：`docs/client-config-prototype.html`（注：原型开关映射已过时，以本文 §2.4 为准）。

## 1. 决策与约束

### 1.1 用户目标

建完 key 那一刻最需要知道「下一步粘到哪、怎么粘」。本功能把「拿到 key」→「客户端跑起来」之间的手工拼配置抹掉。

### 1.2 核心行为

- 选 API Key（下拉，来自 `GET /api/v1/admin/api-keys`，明文字段 `api_key`）+ 客户端类型（Claude Code / Codex）
- Claude Code：三档位（sonnet/opus/haiku）各自下拉选 `group.name`（来自 `GET /api/v1/admin/groups`）+ 三个可选开关
- Codex：单默认模型下拉选 `group.name`
- 网关地址默认 `window.location.origin`，可手填覆盖
- 表单任意改动 → 配置预览实时刷新（只读）
- 复制 / 下载生成物；折叠的「原始 JSON」给高级用户手改兜底
- 「如何安装」手风琴，按客户端类型给「粘到哪个文件 / 已有配置怎么合并」

### 1.3 成功标准（可验证）

| # | 场景 | 期望 |
|---|---|---|
| S1 | 选 Claude Code + 填好 key/三档位/网关 | 预览输出合法 JSON，含 §2.4 Claude Code 全部勾选字段 |
| S2 | 勾 hideAttribution | JSON 顶层出现 `"attribution": {"commit":"","pr":""}` |
| S3 | 勾 effortMax | JSON 顶层出现 `"effortLevel": "high"` |
| S4 | 勾 disableAutoUpdate | `env` 内出现 `"DISABLE_AUTOUPDATER": "1"` |
| S5 | 切到 Codex | 预览输出 config.toml 段 + .env 段，含 `env_key = "GALAXY_API_KEY"` |
| S6 | 三个开关全不勾 | JSON 只剩 `env`（base_url/api_key/三档位），无顶层 attribution/effortLevel |
| S7 | 创建 key 成功 | 成功视图出现「生成客户端配置」入口，点进去 key 已预填 |
| S8 | 侧边栏「客户端配置」 | 跳内部 `/client-config`，不再外链 GitHub |
| S9 | HTTP 部署点复制 | 走 `copyText` 降级，不抛 `Cannot read properties of undefined` |

### 1.4 明确不做（可 grep 反向核对）

- ❌ 后端生成接口（grep `client-config` 在 `src/api/handlers/admin/` 应无新增 route）——纯前端模板拼接
- ❌ 「一键应用」配置到用户本机（服务端不可能）
- ❌ 右侧可编辑 JSON 编辑器（只读预览 + 折叠原始 JSON，grep 不应出现双向绑定的 JSON editor 组件）
- ❌ Cursor / Cline（只 Claude Code + Codex）
- ❌ key 持久化到 localStorage（刷新即丢，与现状一致不引入新风险）
- ❌ 把 key 写进 Codex `auth.json`（用官方 env_key 方式，见 §2.4）

---

## 2. 名词层 + 编排层

### 2.1 名词层

| 术语 | 含义 |
|---|---|
| group.name | galaxy 的虚拟模型名。一个 group = 一个对外模型，按 priority/weight 路由到真实 channel。Claude Code 档位 / Codex 默认模型下拉的选项就是它 |
| 三档位 | Claude Code 的 sonnet / opus / haiku 三个模型档位，分别可指向不同 group |
| settings.json | Claude Code 用户级配置文件 `~/.claude/settings.json` |
| config.toml / .env | Codex 配置 `~/.codex/config.toml` + 凭证环境 `~/.codex/.env` |

### 2.2 现状（挂载点清单）

| 位置 | 现状 |
|---|---|
| `frontend/src/App.tsx:106-116` | 路由平铺区，`/client-config` 加在这 |
| `frontend/src/pages/index.ts:1-14` | 页面 re-export |
| `frontend/src/components/layout/Sidebar.tsx:119-133` | 「客户端配置」当前是 `<a href={DOCS_URL}>` 外链 GitHub |
| `frontend/src/pages/ApiKeys.tsx:94-113` | `handleCreate` 创建成功 → `setNewKeyResult(key)` + toast |
| `frontend/src/components/ApiKeyForm.tsx:82-114` | 创建成功视图（绿面板显示明文 key + 复制 + 我已保存） |
| `frontend/src/lib/utils.ts:48-73` | `copyText(text): Promise<boolean>`，含 execCommand 降级 |
| `frontend/src/api/query-hooks.ts:54-56` / `84-86` | `useGroups()` / `useApiKeys()` |
| `frontend/src/api/types.ts:151-162` / `199-210` | `Group.name` / `ApiKey.api_key`（明文） |

### 2.3 变化

**新增**
- `frontend/src/lib/clientConfig.ts` — 纯函数 `generateClaudeConfig(input)` / `generateCodexConfig(input)`，可单测
- `frontend/src/pages/ClientConfig.tsx` — 单栏流式 UI 页
- 「如何安装」手风琴（Claude Code / Codex 两套指引），直接写在 `ClientConfig.tsx` 内，不单独抽组件

**改动**
- `App.tsx` — 注册 `<Route path="client-config">`
- `pages/index.ts` — re-export `ClientConfig`
- `Sidebar.tsx:119-133` — `<a href={DOCS_URL}>` → `<Link to="/client-config">`，图标保留或换 `MonitorSmartphone`
- `ApiKeyForm.tsx:82-114` 成功视图 — 加「生成客户端配置」按钮，点击 `onGenerateConfig(key)` 回调 → 父组件 `navigate('/client-config', { state: { apiKey: key.api_key } })`

**不动**：后端、`api/`、`types.ts`、其他页面

### 2.4 字段映射（核心，已官方核实）

#### Claude Code → `~/.claude/settings.json`

来源：[code.claude.com/docs/en/settings](https://code.claude.com/docs/en/settings)（2026-07-02 抓取）

```jsonc
{
  "env": {
    "ANTHROPIC_BASE_URL": "<网关地址>",               // 官方 env（env-vars 页）
    "ANTHROPIC_API_KEY": "<api_key>",                 // 官方 env（env-vars 页）
    "ANTHROPIC_DEFAULT_SONNET_MODEL": "<group.name>", // 官方 env（env-vars 页确认，Model configuration）
    "ANTHROPIC_DEFAULT_OPUS_MODEL": "<group.name>",   // 同上
    "ANTHROPIC_DEFAULT_HAIKU_MODEL": "<group.name>",  // 同上
    "DISABLE_AUTOUPDATER": "1",                       // 仅 disableAutoUpdate 勾选（env-vars 页）
    "CLAUDE_CODE_EFFORT_LEVEL": "max",                // 仅 effortMax 勾选；env 支持 max（顶层 effortLevel key 仅到 xhigh）
    "CLAUDE_CODE_ALWAYS_ENABLE_EFFORT": "1"           // 仅 effortMax 勾选；galaxy 用自定义 model ID，不加则 effort 参数不下发（静默失效）
  },
  "attribution": { "commit": "", "pr": "" }           // 仅 hideAttribution 勾选；空串隐藏署名
}
```

**开关 → 字段对照**（⚠️ 注：原型里三开关映射均有误，此处按官方文档纠正：hideAttribution 是顶层 key，effortMax/disableAutoUpdate 是 env）

| UI 开关 | 生成字段 | 类型 | 出处 |
|---|---|---|---|
| hideAttribution（隐藏署名） | `attribution: {commit:"", pr:""}` | 顶层 key | Attribution settings 节 |
| effortMax（最高思考强度） | `env.CLAUDE_CODE_EFFORT_LEVEL: "max"` + `env.CLAUDE_CODE_ALWAYS_ENABLE_EFFORT: "1"` | env | env-vars 页；ALWAYS_ENABLE_EFFORT 因 galaxy 走自定义 model ID，确保 effort 参数实际下发（否则静默失效） |
| disableAutoUpdate（禁用自动更新） | `env.DISABLE_AUTOUPDATER: "1"` | env | autoUpdatesChannel 行 |

> effortMax 用 env `CLAUDE_CODE_EFFORT_LEVEL: "max"`（env-vars 页确认支持 max；顶层 effortLevel key 只到 xhigh，无法表达"最顶"）。官方注："Available levels depend on the model"——非 Claude 上游可能忽略 effort 参数，不影响功能。

#### Codex → `~/.codex/config.toml` + `~/.codex/.env`

来源：[developers.openai.com/codex/config-advanced](https://developers.openai.com/codex/config-advanced) + [morphllm.com/codex-provider-configuration](https://www.morphllm.com/codex-provider-configuration)（custom provider 标准做法）

`config.toml`：
```toml
model_provider = "galaxy"
model = "<group.name>"

[model_providers.galaxy]
name = "Galaxy Router"
base_url = "<网关地址>/v1"
env_key = "GALAXY_API_KEY"
wire_api = "responses"
```

`.env`（放 `~/.codex/`，Codex 自动 dotenv 加载）：
```
GALAXY_API_KEY=<api_key>
```

**关键**：custom provider 不从 `auth.json` 读 key，而是读 `env_key` 指定的环境变量。所以 key 走 `.env`（或用户 shell export），**不写 auth.json**。`wire_api = "responses"` —— 最新版 Codex 默认走 Responses API，galaxy 的 `/v1/responses`（`responses::proxy`）兼容。base_url 仍为 `<网关>/v1`，Codex 自动拼 `/responses`。

> 已定（2026-07-02 用户确认）：`wire_api = "responses"`（最新版 Codex 默认 Responses API，galaxy `/v1/responses` 兼容）。

---

## 3. 验收契约（`clientConfig.ts` 纯函数）

输入类型：
```ts
interface ClaudeInput {
  baseUrl: string; apiKey: string;
  sonnet: string; opus: string; haiku: string; // group.name
  hideAttribution?: boolean; effortMax?: boolean; disableAutoUpdate?: boolean;
}
interface CodexInput { baseUrl: string; apiKey: string; model: string; } // group.name
```

| 用例 | 输入 | 期望输出 |
|---|---|---|
| C1 | ClaudeInput 全填、三开关关 | `{env:{ANTHROPIC_BASE_URL,ANTHROPIC_API_KEY,ANTHROPIC_DEFAULT_SONNET_MODEL,ANTHROPIC_DEFAULT_OPUS_MODEL,ANTHROPIC_DEFAULT_HAIKU_MODEL}}` |
| C2 | C1 + hideAttribution | 顶层多 `attribution:{commit:"",pr:""}` |
| C3 | C1 + effortMax | `env` 多 `CLAUDE_CODE_EFFORT_LEVEL:"max"` + `CLAUDE_CODE_ALWAYS_ENABLE_EFFORT:"1"` |
| C4 | C1 + disableAutoUpdate | `env` 多 `DISABLE_AUTOUPDATER:"1"` |
| C5 | C1 + 三开关全开 | `env` 含 DISABLE_AUTOUPDATER + CLAUDE_CODE_EFFORT_LEVEL + CLAUDE_CODE_ALWAYS_ENABLE_EFFORT，顶层含 attribution |
| D1 | CodexInput 全填 | config.toml 段含 `model_provider="galaxy"` + `[model_providers.galaxy]` 四键；.env 段 `GALAXY_API_KEY=<api_key>` |
| D2 | baseUrl 带/不带尾斜杠 | `base_url` 恒为 `<baseUrl 去尾斜杠>/v1` |

`generateClaudeConfig` 返回对象（`JSON.stringify` 后展示）；`generateCodexConfig` 返回 `{ files: [{path, content}] }`，UI 把 files 拼成带 `# <path>` 注释的多段文本展示。

---

## 4. 推进策略

### Phase A：纯函数（退出信号：tsc 通过 + §3 契约人工核对）

1. **核实三档位 env 变量名** — fetch `code.claude.com/docs/en/env-vars` 确认 `ANTHROPIC_DEFAULT_{SONNET,OPUS,HAIKU}_MODEL` 存在 ✅（已完成，见 task notes；附带发现 effortMax 需配 ALWAYS_ENABLE_EFFORT）
2. `frontend/src/lib/clientConfig.ts`：`generateClaudeConfig` / `generateCodexConfig`，纯函数（结构上可单测）
3. §3 契约（C1-C5, D1-D2）人工核对 —— 项目前端**无测试框架**（`package.json` 仅 dev/build/lint/preview），不为此功能引入 vitest；靠 `tsc -b`（含在 build）+ 手动核对契约

### Phase B：UI 页（退出信号：手动走通 S1-S6）

4. `ClientConfig.tsx`：Tab + key 下拉 + base_url + 三档位/Codex 单模型 + 三开关 + 实时预览 + 复制/下载 + 安装手风琴
5. `App.tsx` 注册路由 + `pages/index.ts` re-export

### Phase C：接入触发点（退出信号：S7-S8）

6. `ApiKeyForm.tsx` 成功视图加「生成客户端配置」按钮 → `navigate('/client-config',{state:{apiKey}})`
7. `ClientConfig.tsx` 读 `useLocation().state.apiKey` 预填
8. `Sidebar.tsx` 外链 → 内部 `<Link>`

### Phase D：验收（退出信号：§1.3 全绿 + lint/test 通过）

9. `pnpm lint` + `pnpm build`（含 `tsc -b` 类型检查）全绿
10. 手动核对 §1.3 九条 + chrome-devtools 实测复制（含 HTTP 降级路径，呼应 pitfalls）
11. 写 report.md

---

## 风险与缓解

| 风险 | 缓解 |
|---|---|
| 三档位 env 变量名过时 | Phase A step 1 官方页二次核；字段集中在 clientConfig.ts 一处易改 |
| `window.location.origin` 不准（反代/内网） | 字段可手填覆盖 |
| Codex `.env` 方式用户不熟 | 安装指引明确「放到 ~/.codex/.env 或 export」，给两路 |
| effort 参数上游不兼容 | galaxy 透传 effort 到上游；非 Claude 模型可能忽略，不影响功能（官方语 "Available levels depend on the model"） |
