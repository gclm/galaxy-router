# Relay 与 Scheduler 独立模块拆分决策

## 背景

Galaxy Router 正在从单一 `proxy` 模块演进为“Octopus 式 Relay Pipeline + Sub2API 式自动负载均衡/监控统计”的组合架构。原先 `proxy/` 同时承担 HTTP 代理入口、渠道选择、协议转换编排、请求执行、熔断、统计等职责，导致：

- 转换链路散落在 `prepare.rs` / `execute.rs` / `protocol/*` 多处。
- 调度逻辑与执行逻辑耦合，后续 hard capacity、attempt trace、top-K scoring 难以清晰落地。
- 原 AGENTS 约束“不新建独立 scheduler 目录”已不适合当前重构方向。

## 决策

从本次重构开始，允许并优先使用根级新模块：

- `src/relay/`：负责一次代理请求生命周期、attempt 生命周期、统一 pipeline、metrics 汇总。
- `src/scheduler/`：负责候选排序、评分、top-K、weighted order、capacity permit、attempt trace、sticky/circuit 调度决策。

`src/proxy/` 进入过渡角色：保留现有外部入口与兼容逻辑，后续逐步瘦身为 relay 的适配层。

## 理由

1. **职责边界更清晰**：Relay 管请求生命周期，Scheduler 管候选和准入，Protocol 管协议编解码。
2. **符合参考项目取舍**：吸收 Octopus 的 `relayRun/relayAttempt` 边界，吸收 Sub2API 的 scheduler scoring/capacity 设计。
3. **更适合 TDD**：`scheduler` 可先作为纯逻辑模块独立测试，不依赖 HTTP/DB。
4. **避免继续扩大 proxy 神模块**：当前 `proxy/mod.rs` 和 `proxy/execute.rs` 已经职责过重。

## 约束

- 不新建全局 `src/error.rs`；错误类型仍放在各模块内部。
- `protocol/inbound/` 和 `protocol/outbound/` 空目录仍不使用。
- 迁移必须走 TDD：新模块每个切片先 RED 再 GREEN。
- 迁移期间保持 `/v1/*` 外部协议格式不变。
- 迁移期间 `proxy` 可 re-export 或转调新模块，直到旧链路清理完成。

## 影响

- AGENTS.md 中关于 scheduler 目录的旧警告废止。
- `.gclm-harness/decisions/architecture/02-module-design.md` 需要更新模块图与职责表。
- 当前已实现的 `proxy::scheduler::scoring/capacity` 将迁移到 `scheduler::scoring/capacity`。
