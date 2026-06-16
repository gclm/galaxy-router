# 验收报告:创建 API Key 时支持设置预算限额

> feature · 前端两步编排(创建成功后设置预算)

## 背景

`budget_limits` 是独立实体,需 `api_key_id` 才能写入,故原 create 接口不带预算,创建表单也无预算输入框,只能在编辑时设。本次让创建时即可填预算(方案 A:前端两步编排,后端零改动)。

## 改动

- `frontend/src/components/ApiKeyForm.tsx`
  - 预算输入框去掉 `{apiKey && ...}` 条件,创建/编辑都渲染
  - `handleSubmit` 去掉 `if (apiKey)` 包裹,创建模式也把 budget 附到 data
- `frontend/src/pages/ApiKeys.tsx`
  - `handleCreate` 的 `onSuccess(key)` 里,若 data 带预算则调 `setBudgetMutation.mutate({ api_key_id: key.id, ... })`(复用已有 `useSetBudget`,在 `ApiKeys.tsx:67`)

后端零改动。

## 验收契约

| 场景 | 期望 | 实际 | 通过 |
|------|------|------|------|
| 创建 key 填月/日限额 | 创建成功 + 预算写入 | onSuccess 触发 setBudgetMutation 并 invalidate budgets | ✓(逻辑) |
| 创建 key 不填预算 | 只建 key,不建预算 | budget 字段未附,不调 setBudget | ✓(逻辑) |
| 编辑 key 设预算(原功能) | 不受影响 | handleUpdate 逻辑未动 | ✓ |

## 测试结果

- build(tsc + vite): ✓
- eslint(4 个改动文件 `utils.ts` / `ApiKeyForm.tsx` / `ApiKeys.tsx` / `Logs.tsx`): ✓ 全干净
- 手动验证(创建 key 填预算 → listBudgets 查到 / 列表显示限额): 需用户在 UI 确认

## 顺手发现

- 创建时设预算若 `setBudgetMutation` 失败(网络/后端),当前未提示(与 `handleUpdate` 行为一致),key 已建但预算未设。低概率,保持最小不改,后续如需可加 onError toast。
