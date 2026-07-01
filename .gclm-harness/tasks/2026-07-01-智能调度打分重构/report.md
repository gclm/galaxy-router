---
doc_type: report
feature: 2026-07-01-智能调度打分重构
status: completed
---

# 智能调度打分重构 实现报告

## 根因回顾

线上 glm-5.2 全走智谱、商汤（新渠道）0 次。排查定位到 scheduler 打分层 4 个结构性失效（非重试/熔断 bug）：priority 权重失效、latency 打分失效（有数据就满分）、冷启动埋没新渠道、并发分散失效（load/queue 双失效）。

## 完成改动（P1-P4）

**P1 打分层** — `scheduler/scoring.rs` + `relay/candidates.rs` + `scheduler/capacity.rs`
- 打分公式 6 因子 → 4 因子：`latency（候选间相对归一化）+ least_conn（新增）+ error_rate + health`，移除 priority/load/queue
- latency 冷启动 0.8（不埋没新渠道）
- capacity.rs 加 `active_count(channel_id)`，最小连接数基于实时在途并发
- scoring.rs 新增 5 个内联测试

**P2 候选层** — `relay/candidates.rs`
- priority 硬分档（BTreeMap 按 priority 升序）+ 档内打分 + `tier_and_demote`（health<=0 自动沉底）
- 提取纯函数 `tier_and_demote` 便于单测，新增 3 个测试

**P3 前端** — `GroupForm` / `Groups` / `ChannelForm`
- 隐藏重试配置（retry_enabled/max_retries UI），priority input 加说明文案
- Groups 重试列改"自动"；ChannelForm max_concurrency 标注「可选，0=不限」

**P4 DB 清理** — 迁移 `15_drop_unused_fields.sql`
- max_retries 全链路移除（admin groups.rs + backup.rs + 前端 types + GroupForm 提交）
- DROP `groups.max_retries` + `channels.custom_headers` + `channels.extras`（custom_headers/extras 是 commit 5438140 迁移后的孤儿列）
- 重写 `tests/scheduler_scoring_tdd.rs`（旧 priority/load 语义 → 新 least_conn/error/latency 语义）
- 修复 `tests/integration_test.rs` fixture（custom_headers 迁移到 endpoint.headers）

## 验证

- `cargo test` 全量通过：lib 260+266，集成 94+26+19+52+20+5+9+5+3+2+19，**0 failed**
- `pnpm build` 通过
- `cargo check` 通过（1 个 dead_code warning，见遗留）

## 遗留（提一嘴，未在本 feature 清理）

1. **`load_rate` / `active_requests` / `increment_active` 成 dead code**（state.rs）：load 维度被 least_conn 取代后，这套为 load_rate 服务的字段/方法失去用途，产生 1 个 warning。属本 feature 副产品，可顺手清理（删方法 + `load_rate_is_zero_when_no_max_concurrency` 测试），但未在 design 范围，建议单独小 task。
2. **`CreateChannelRequest.custom_headers`**（admin types）：commit 5438140 迁移后该字段接收前端旧数据但 crud.rs 不存，是死字段。不影响功能。

## 上线注意

- **迁移 15 DROP 三列（不可逆）**，需 `make brew-deploy` 重启服务生效
- 打分权重默认值（latency 1.0 / least_conn 1.0 / error_rate 1.5 / health 1.0）是起点，上线后按 metrics（各渠道选中比例、延迟分布、新渠道上场率）调参
- 想优先用某渠道（包月/测试）：在 group 里设该渠道 priority=1（数值小=高档先选），档内仍按速度打分
