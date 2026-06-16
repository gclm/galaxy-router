# 问题报告:复制 API Key 在非安全上下文(HTTP)下报错

> 前端 · 复制降级兜底

## 现象

云端 HTTP 部署下,点击「复制 API Key」抛错:
`Uncaught (in promise) TypeError: Cannot read properties of undefined (reading 'writeText')`
本地 localhost 正常。

## 根因

- **位置**:`frontend/src/pages/ApiKeys.tsx:88`、`frontend/src/pages/Logs.tsx:35`
- **原因**:`navigator.clipboard`(Web Clipboard API)只在安全上下文(HTTPS / localhost / file://)下存在;HTTP 部署下为 `undefined`,直接调 `.writeText` 即抛上述错误。本地用 localhost 属安全上下文,故不复现。

## 期望 vs 实际

- 期望:HTTP 部署下点击复制也能写入剪贴板(或至少不崩)
- 实际:`navigator.clipboard` 为 undefined → 抛 TypeError,复制按钮失效

## 修复(方案 B,已完成 ✅)

新增 `frontend/src/lib/utils.ts::copyText(text)`:优先 `navigator.clipboard.writeText`,不可用或失败时降级到 `document.execCommand('copy')`(临时 textarea + select)。`ApiKeys.tsx` / `Logs.tsx` 两处复制改为调用它,返回值不抛异常。

方案 A(部署侧上 HTTPS)属运维范围,不在本次。

### 验收

- `pnpm build`(tsc type-check + vite build):通过
- `eslint` 本次改动文件(`utils.ts` / `Logs.tsx`):干净
- 真实 HTTP 场景验证:需在云端 HTTP 环境点击确认(本地 localhost 无法复现非安全上下文)

### 测试

- 前端无测试框架(见「顺手发现」),本次按 type-check + build + lint + 手动验证验收。

## 顺手发现

- `frontend/src/pages/ApiKeys.tsx:95` `budget_monthly` / `budget_daily` 解构后未用(`no-unused-vars`)—— 预存代码(handleCreate 本就解构丢弃),非本次引入。下一个 feature 任务(创建时设预算)改 handleCreate 时会用到,届时自然消除。
- 前端整体无测试框架(`package.json` 无 vitest/jest,scripts 无 `test`);全量 `eslint` 有 31 个预存问题(`Settings.tsx` 等 `react-hooks/set-state-in-effect`)。建议后续单开 feature 引入 vitest + 清理预存 lint。
