# 开发路线图

> 跨任务的开发计划和优先级。单个任务的计划放在 `tasks/{slug}/design.md` 里，这里只放全局视角的规划和已知的下一步。

## 当前阶段

**目标**：核心代理功能稳定可用，回归测试覆盖完整，准备进入功能增强阶段。

## 进行中

无

## 近期计划

- [x] Admin API RESTful 规范化（优先级：P1）— 路径/方法对齐 RESTful 约定，前后端联动
- [x] 错误类型统一迁移到 `src/error/`（优先级：P2）— `ApiError` + `ProxyError` 已迁入 `error/app.rs` + `error/proxy.rs`
- [ ] 使用 `data/galaxy.db` 的 E2E 测试（优先级：P2）— 补充真实数据端到端验证
- [ ] 前端管理后台 UI 完善（优先级：P2）

## 已知技术债

无

## 已完成（最近 10 条）

- ✅ 2026-06-10 refactor: Admin API RESTful 规范化 — 前后端路径同步
- ✅ 2026-06-10 refactor: 错误类型统一迁移到 `src/error/` — ApiError + ProxyError
- ✅ 2026-06-10 回归测试全套：92 API + 20 代理全链路用例
- ✅ 2026-06-10 fix: backup fetch_channels SQL 补全缺失列
- ✅ 2026-06-10 feat: health 接口增加 uptime_seconds
- ✅ 2026-06-10 冷启动知识层生成（Phase 2-4）
- ✅ 2026-06-10 fix: 协议转换全路径打通 — 端点选择 + null 字段修复
- ✅ 2026-06-10 refactor: 拆分 pipeline.rs → converter + candidates + pipeline
- ✅ 2026-06-10 refactor: 拆分 relay/state.rs → error + state + pipeline
- ✅ 2026-06-10 refactor: 清理 scheduler/ 26 个 dead_code 注解
- ✅ 2026-06-10 refactor: Phase 4+5 — 执行逻辑迁入 relay/
- ✅ 2026-06-10 refactor: Phase 1-3 — 文件迁移 + protocol 拆分 + scheduler 接入
