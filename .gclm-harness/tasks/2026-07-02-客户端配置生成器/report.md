# 验收报告 — 客户端配置生成器

> 2026-07-02 联调通过。纯前端功能，无后端改动。

## 验收契约核对（design §1.3 + §3）

| 场景 | 期望 | 实测 | 通过 |
|---|---|---|---|
| S1 Claude Code 全填三开关关 | env 五字段 | env 五字段，无顶层 | ✓ |
| S2 +hideAttribution | 顶层 `attribution:{commit:"",pr:""}` | 同 | ✓ |
| S3 +effortMax | env `CLAUDE_CODE_EFFORT_LEVEL:"max"` + `CLAUDE_CODE_ALWAYS_ENABLE_EFFORT:"1"` | 同 | ✓ |
| S4 +disableAutoUpdate | env `DISABLE_AUTOUPDATER:"1"` | 同 | ✓ |
| S5 Codex | config.toml + .env，`wire_api=responses` | 同 | ✓ |
| S6 三开关全关 | 只 env | 只 env | ✓ |
| S7 创建 key 入口 | 成功视图按钮 → 跳转 + key 预填 | 跳 `/client-config`，新建 `sk-gr-...` 自动选中 | ✓ |
| S8 侧边栏 | 内部 `/client-config` | `<Link>` 内部路由，激活态正常 | ✓ |
| S9 复制 | copyText 工作 | toast「已复制到剪贴板」 | ✓ |
| C1-C5 函数契约 | 见 design §3 | 全部符合 | ✓ |
| D1 Codex 全字段 | model_provider/base_url/env_key/wire_api | 全有 | ✓ |
| D2 base_url 去尾斜杠 | `+/v1` | `http://localhost:5173/v1` | ✓ |

## 测试结果

- `tsc -b`: ✓
- `pnpm build`: ✓（2481 modules，221ms）
- 本次改动 lint: ✓（0 新增 error）
- chrome-devtools 联调: ✓（S1-S9 + C1-C5 + D1/D2 全通过）

## 挂载点检查

- `frontend/src/lib/clientConfig.ts`: ✓ 新增（纯函数）
- `frontend/src/pages/ClientConfig.tsx`: ✓ 新增（单栏 UI）
- `App.tsx`: ✓ 注册 `/client-config` 路由
- `pages/index.ts`: ✓ re-export
- `Sidebar.tsx`: ✓ 外链 GitHub → 内部 `<Link>`；孤儿 `DOCS_URL` 已删
- `ApiKeyForm.tsx`: ✓ 成功视图加「生成客户端配置」按钮
- `ApiKeys.tsx`: ✓ `useNavigate` + `onGenerateConfig` 传 `{state:{apiKey}}`

## 字段映射核实依据

Claude Code: [code.claude.com/docs/en/settings](https://code.claude.com/docs/en/settings) + [/env-vars](https://code.claude.com/docs/en/env-vars)
Codex: [developers.openai.com/codex/config-advanced](https://developers.openai.com/codex/config-advanced)

关键修正（核实推翻原型假设）：
- hideAttribution → 顶层 `attribution`（非 env `CLAUDE_CODE_HIDE_ATTRIBUTION`）
- effortMax → env `CLAUDE_CODE_EFFORT_LEVEL:"max"`（顶层 `effortLevel` 无 max 值）+ `CLAUDE_CODE_ALWAYS_ENABLE_EFFORT:"1"`（galaxy 用自定义 model ID，不加则 effort 静默失效）
- Codex key → `env_key` 指环境变量放 `.env`（custom provider 不读 auth.json）

## 顺手发现（非本次引入，建议后续处理）

1. **`vite.config.ts` proxy 前缀过宽**：`'/api'`（无尾斜杠）匹配前端路由 `/api-keys`、`/api-key-stats`，dev 下直接 navigate 这两个路由会被转发到后端 8080 → 404/白屏。修复：proxy key 改 `'/api/'`、`'/v1/'`（带尾斜杠）。client routing（侧边栏点击）不受影响，故**生产无碍**，仅影响 dev 下直接刷这两个 URL。
2. **全项目 21 个 `react-hooks/set-state-in-effect` lint error**（Settings.tsx、App.tsx NavigationProgress 等），pre-existing。用户曾要求修复，后转联调，未做。
3. 联调新建了一个测试 key「联调测试key」(`sk-gr-019f223a-...`)，留在 `data/galaxy.db`，可手动删除。
