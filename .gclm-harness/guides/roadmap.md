# 开发路线图

> 跨任务的开发计划和优先级。单个任务的计划放在 `tasks/{slug}/design.md` 里，这里只放全局视角的规划和已知的下一步。

## 当前阶段

**目标**：核心代理功能稳定可用，回归测试覆盖完整，准备进入功能增强阶段。

## 进行中

无

## 近期计划

无

## 已知技术债

- `ApiKeys.tsx` 表单逻辑内联（~680 行），后续维护时建议抽成 `ApiKeyForm` 组件

## 已完成（最近 17 条）

- ✅ 2026-06-16 refactor: 仪表盘 stats 查询命中 created_at 索引 — `range_utc_*` helper + 9 处 WHERE 改裸列,overview/daily/latency 从 pending→200
- ✅ 2026-06-16 fix: budget 月消费误算误拦 + 无法删除预算 — check_budget monthly 加当月过滤 + handleSubmit 总附 budget(含0)
- ✅ 2026-06-16 fix: 复制 API Key 在 HTTP 非安全上下文崩溃 — `utils.ts::copyText` 加 execCommand 降级(`ApiKeys.tsx`/`Logs.tsx`)
- ✅ 2026-06-16 feat: 创建 API Key 时支持设置预算限额 — 前端两步编排(create→onSuccess 调 setBudget),后端零改
- ✅ 2026-06-16 feat: SSE 流内错误状态码归因(502→429) — 限流不触发 channel 黑名单(阶段 2,多 key 重试改造收尾)
- ✅ 2026-06-16 feat: per-key 熔断 — circuit_breaker per-key + 黑名单 channel 级,单 key 受限不再连累整渠道(阶段 1)
- ✅ 2026-06-16 fix: glm-5.2 限流不换 key — `KEY_NEEDLES` 补中文限流词(速率限制/频率限制),多 key 重试健壮性改造 阶段 0 止血
- ✅ 2026-06-10 refactor: 前端全面优化 — 15 条审计问题修复 + SortHeader/DataTable 通用组件抽取
- ✅ 2026-06-10 refactor: Admin API RESTful 规范化 — 前后端路径同步
- ✅ 2026-06-10 refactor: 错误类型统一迁移到 `src/error/` — ApiError + ProxyError
- ✅ 2026-06-10 回归测试全套：92 API + 20 代理全链路用例
- ✅ 2026-06-10 fix: backup fetch_channels SQL 补全缺失列
- ✅ 2026-06-10 feat: health 接口增加 uptime_seconds
- ✅ 2026-06-10 冷启动知识层生成（Phase 2-4）
- ✅ 2026-06-10 fix: 协议转换全路径打通 — 端点选择 + null 字段修复
- ✅ 2026-06-10 refactor: 拆分 pipeline.rs → converter + candidates + pipeline
- ✅ 2026-06-10 refactor: 拆分 relay/state.rs → error + state + pipeline
