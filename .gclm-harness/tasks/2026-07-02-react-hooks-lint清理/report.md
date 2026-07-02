# 验收报告 — react-hooks lint 清零

## 结果
- **28 problems → 0**（19 errors + 9 warnings 全清）
- tsc ✓ / build ✓（224ms）

## 修复明细

| 规则 | 数量 | 文件 | 修法 |
|---|---|---|---|
| `react-hooks/refs` | 13 | Playground(12) useTableLoader(1) | saved `useRef(loadConfig())`→`useState(loadConfig)` lazy init；fetchFnRef render 写→effect 写 |
| `react-hooks/set-state-in-effect` | 6 | App/Settings/Playground/useTableLoader | Playground `selectedApiKeyId` state→derived（一并消 set-state + exhaustive-deps）；其余合理副作用 disable+注释 |
| `react-hooks/exhaustive-deps` | 9 | ApiKeyStats/ChannelStats/ModelStats/Dashboard | `stats`/`rawDaily` 逻辑表达式包 `useMemo` 稳定引用 |
| `react-hooks/purity`（暴露） | 2 | Playground | handleSend 内 `Date.now`（事件处理器计时），规则误报，disable+注释 |

## 真修 vs disable

**真修（消除 anti-pattern）**：
- Playground saved：ref→state lazy init
- Playground selectedApiKeyId：effect setState→derived state
- useTableLoader fetchFnRef：render 写 ref→effect 写
- 4 个 Stats/Dashboard：逻辑表达式→useMemo

**disable（合理副作用/规则误报，5 处，均注释原因）**：
- Playground handleSend `Date.now` ×2 — 事件处理器计时，purity 规则对组件内 async 函数误报
- App NavigationProgress `setActive` — 路由响应进度条，动画时机依赖 state 切换，CSS key 真修会改视觉
- Settings CorsTab `setSavedValue` — 外部 corsValue 同步本地乐观副本，key 真修需改父组件
- useTableLoader `setLoading`/`setPage` ×3 — 公共 hook 的数据加载 + 页码重置，重构需改公共 API 影响多页

## 顺手发现
build 生成的 `dist/` 会干扰 vite dev（SPA fallback 拿到 `dist/index.html` 导致 `/api-keys` 等路由白屏，见上个任务 report）。本次 build 又生成了 dist，dev 前需删。
