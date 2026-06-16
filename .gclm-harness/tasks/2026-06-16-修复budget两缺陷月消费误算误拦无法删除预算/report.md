# 问题报告:budget 月消费误算误拦 + 无法删除预算

## 现象
1. 设预算后"一设就被拦":即使当月消费未超,历史消费被计入月预算导致 402。
2. 清空预算保存后预算仍在:无法删除预算。

## 根因
1. **月消费误算**:`src/relay/pipeline.rs::check_budget` 的 monthly_cost 查询 `SUM(cost) FROM usage_logs` **无当月过滤**,把全部历史消费当成月消费(日消费查询有 `date(created_at)=date('now')`,唯独月漏了)。
2. **无法删除**:`frontend/src/components/ApiKeyForm.tsx::handleSubmit` 仅 `monthly>0||daily>0` 时附 budget 字段,清空(=0)时不附 → `handleUpdate` 的 `budget_monthly !== undefined` 为 false → 走不进 deleteBudget 分支。

## 修复
- 后端 `check_budget`:monthly 加 `strftime('%Y-%m', created_at) = strftime('%Y-%m','now')` 当月过滤。
- 前端 `handleSubmit`:总是附 budget 字段(含 0);`handleCreate` 加 `>0` 守卫避免给新 key 写 0 记录。`handleUpdate` 原有 `=0→deleteBudget` 即生效。

## 验收(E2E + 单测)
- cargo 回归测试 2 个(`pipeline::tests`):跨月不计入 / 当月超额仍拦。全 229 测试过。
- E2E(brew-deploy v1.0.1):场景1 当月 3000≥2323 → **402**;场景2 当月 100<2323 + 上月历史 5000 → **非 402**(历史不计入)。
- 删除:正确清空(React valueTracker)→ `deleteBudget` → `budget_limits` 记录删除。

## 顺手发现
- chrome-mcp `fill` 对 React 受控 input 无效(只改 DOM value,不触发 onChange,React valueTracker 不感知)。测 React 表单需用 `evaluate_script` 走 valueTracker + dispatch input event——这导致问题1 的首次"chrome 复现"是假象。

## 补充修复:无消费时 SUM 解码失败

首版上线后发现:`check_budget` 消费查询 `SUM(...)` 在该 key **无消费记录**时返回 INTEGER 0,sqlx 按 `f64` 解码失败("Rust f64 not compatible with SQL INTEGER"),误报 402 "查询消费失败"。根因:首版修复漏了 `CAST AS REAL`(`metrics/query` stats 查询有,`check_budget` 没有)。
- 补丁:两列加 `CAST(... AS REAL)` + 新增 `check_budget_no_usage_decodes_cleanly` 回归测试。
- **为什么首次没排查出**:限额 E2E 和 cargo 回归都注入 REAL cost,SUM 返回 REAL、decode 正常,从没覆盖"无消费 → INTEGER 0"分支。
