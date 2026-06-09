# Proxy 模块退场路线图

> 日期：2026-06-09  
> 状态：draft-for-iteration  
> 前置：`relay-scheduler-tdd-refactor-design.md`、`proxy-relay-migration-checklist.yaml`  
> 目标：把 `src/proxy` 从核心代理实现模块逐步降级为薄 gateway adapter，最终大部分职责迁入 `relay/`、`scheduler/`、`observability/`、`protocol/` 等边界更清晰的模块。

---

## 1. 为什么现在 `src/proxy` 还存在

最初架构目标是：

```text
API handler
  -> RelayRun / RelayStreamRun
  -> Scheduler
  -> RelayAttempt
  -> RelayPipeline
  -> Observability
```

并让：

```text
src/proxy/ = 过渡入口 + 兼容 glue，最终只保留薄封装
```

截至 P10，已经完成的是 **主路径迁移**，不是完整的 **模块归位迁移**。

当前真实主路径已经变为：

```text
非流式：
handle_proxy_request
  -> proxy_request
  -> build_relay_candidates
  -> RelayRun
  -> ProxyRelayExecutor
  -> execute_proxy_request
  -> RelayPipeline

流式：
handle_proxy_request
  -> proxy_stream
  -> build_relay_candidates
  -> RelayStreamRun
  -> ProxyStreamRelayExecutor
  -> execute_proxy_stream
  -> RelayPipeline stream conversion
```

这说明：

- 调度生命周期已经迁到 `relay/`。
- scoring / capacity / trace / runtime / metrics 已迁到 `scheduler/`。
- 请求/响应/流式转换核心已逐步收敛到 `relay::pipeline` + `protocol/`。
- 但 HTTP 执行、SSE 处理、统计落库 glue、状态聚合仍大量留在 `src/proxy`。

因此 `src/proxy` 现在是：

```text
过渡 adapter + 执行遗留 + 观测遗留 + 状态聚合
```

不是最终形态。

---

## 2. 当前已完成里程碑

| 阶段 | 状态 | 说明 |
|---|---|---|
| P1-P2 | done | scheduler scoring / capacity / top-k / weighted order |
| P3 | done | attempt trace builder |
| P4 | done | RelayRun fake executor lifecycle |
| P5 | done | RelayPipeline passthrough / conversion / stream conversion API |
| P8A | done | 非流式 `proxy_request` 外层 retry loop 迁到 `RelayRun` |
| P8B | done | 流式 `proxy_stream` 外层 retry loop 迁到 `RelayStreamRun` |
| P8C | done | 删除旧 selector：`select_channel_for_proxy` / `select_group_item` / `find_channel_by_model` 等 |
| P9 | done | `SchedulerMetrics` 接入生产成功选择反馈 |
| P10 | done | `ChannelRuntimeManager` 接入生产成功/失败/TTFT/monitor 反馈，并参与 scoring |

当前 `make check` 通过。

---

## 3. 当前 `src/proxy` 剩余职责清单

### 3.1 Gateway adapter 职责（短期应保留）

| 文件 | 职责 | 目标 |
|---|---|---|
| `src/proxy/mod.rs` | `/v1/*` 统一入口、auth 后置业务校验、rate limit、budget、错误格式化 | 短期保留，长期可改名为 `gateway/` |
| `src/proxy/mod.rs` | `ProxyError` / `ProxySuccess` | 短期保留，后续评估迁到 gateway error |
| `src/proxy/ratelimit.rs` | API key RPM/TPM 限制 | 可继续作为 gateway concern，或迁入 `auth/limits` |
| `src/proxy/queue.rs` | 全局请求队列 | 可继续作为 gateway/relay shared concern |

### 3.2 Relay 执行职责（应该迁出）

| 文件 | 当前职责 | 目标模块 |
|---|---|---|
| `src/proxy/relay_executor.rs` | `RelayRun` 到真实非流式 HTTP 执行的 adapter | `src/relay/executor.rs` 或 `src/relay/http.rs` |
| `src/proxy/stream_relay_executor.rs` | `RelayStreamRun` 到真实 SSE 执行的 adapter | `src/relay/stream_executor.rs` 或 `src/relay/stream.rs` |
| `src/proxy/execute.rs::execute_proxy_request` | 单次非流式上游 HTTP 请求、响应转换、统计准备 | `src/relay/http.rs` + `observability` |
| `src/proxy/execute.rs::execute_proxy_stream` | 单次流式上游 HTTP 请求、SSE 处理、统计落库 | `src/relay/stream.rs` + `observability` |

### 3.3 Observability 职责（应该迁出）

| 文件 | 当前职责 | 目标模块 |
|---|---|---|
| `src/proxy/execute.rs::AttemptStats` | attempt 统计中间结构 | `src/observability/attempt.rs` 或复用 `scheduler::trace::AttemptTrace` |
| `src/proxy/execute.rs::save_request_record` | request log 落库 glue | `src/observability/recorder.rs` |
| `src/proxy/execute.rs` stream 结束段 | usage/cost/latency/TTFT/response_content 汇总 | `src/observability/usage.rs` + `src/observability/recorder.rs` |
| `src/proxy/sse.rs` 部分 usage/error 工具 | SSE usage/error 提取 | `src/observability/usage.rs` 或 `src/relay/sse.rs` |

### 3.4 Stream/SSE 职责（应该迁出）

| 文件 | 当前职责 | 目标模块 |
|---|---|---|
| `src/proxy/sse.rs` | SSE 边界、usage/error/content 提取、error event 格式化 | `src/relay/sse.rs` 或 `src/stream/sse.rs` |
| `src/proxy/thinking_normalizer.rs` | thinking tag normalizer / extractor | `src/relay/thinking.rs` 或 `src/stream/thinking.rs` |
| `src/proxy/execute.rs` stream loop | upstream stream buffering、转换、背压、drop 后统计 | `src/relay/stream.rs` |

### 3.5 State 职责（最后拆）

| 文件 | 当前职责 | 目标 |
|---|---|---|
| `src/proxy/state.rs::LoadBalancerState` | channel status、circuit、capacity、sticky、metrics、runtime | 拆出 `SchedulerState` 或继续放 `scheduler/state.rs` |
| `src/proxy/cache.rs` | channel/group/model index cache | 可迁到 `channel/cache.rs` 或 `gateway/cache.rs` |
| `src/proxy/mod.rs::ProxyState` | pool/http client/cache/rate limiter/queue/stats/model registry/lb_state/key counter 聚合 | 后期拆成 `GatewayState` + `RelayState` + `SchedulerState` |

---

## 4. 下一阶段迭代路径

### P11：验收与拆分路线图

**目标**：冻结当前中间态，明确后续拆 `proxy` 的路线。

产出：

- 本文档：`proxy-decomposition-roadmap.md`
- checklist 中新增 P11-P16
- 确认当前 `make check` 通过

验收：

```bash
make check
rg "select_channel_for_proxy|select_group_item|find_channel_by_model" src tests
```

---

### P12：把 proxy executor 迁入 relay

**目标**：先迁文件归属，不大改行为。

建议目标结构：

```text
src/relay/
├── executor.rs          # RelayAttemptExecutor 真实实现入口
├── stream_executor.rs   # RelayStreamAttemptExecutor 真实实现入口
├── http.rs              # 单次非流式 HTTP 上游执行
└── stream.rs            # 单次流式 HTTP/SSE 上游执行
```

迁移表：

| 当前 | 目标 |
|---|---|
| `src/proxy/relay_executor.rs` | `src/relay/executor.rs` |
| `src/proxy/stream_relay_executor.rs` | `src/relay/stream_executor.rs` |
| `execute_proxy_request` | `relay::http::execute_once` |
| `execute_proxy_stream` | `relay::stream::execute_once` |

P12 纪律：

- 不改变外部协议。
- 不重写 SSE loop。
- 只做模块归位和命名收敛。
- 每步移动后跑测试。

验收：

```bash
cargo test relay_run --test relay_run_tdd
cargo test sse_integration
make check
```

风险：

- 循环依赖：`relay` 调用 `proxy::ProxyState` 会让边界不清。P12 可以临时保留 `ProxyState` 参数，但要在文档标记为 bridge。
- 可见性调整过大。优先 `pub(crate)`，不要随意 `pub`。

---

### P13：把 request stats glue 迁入 observability

**目标**：把 request log、attempt stats、usage/cost 聚合从 `proxy/execute.rs` 迁走。

建议目标结构：

```text
src/observability/
├── mod.rs
├── attempt.rs       # AttemptStats / attempt -> ChannelAttempt adapter
├── recorder.rs      # save_request_record / stream record helper
├── usage.rs         # usage extraction / token fallback helpers
└── cost.rs          # cost calculation glue（如有必要）
```

迁移表：

| 当前 | 目标 |
|---|---|
| `AttemptStats` | `observability::attempt::AttemptStats` 或复用 `AttemptTrace` |
| `save_request_record` | `observability::recorder::save_request_record` |
| stream 结束统计汇总 | `observability::recorder::record_stream_completion` |
| usage extraction glue | `observability::usage` |

验收：

```bash
cargo test stats::recorder
cargo test sse_integration
make check
```

风险：

- `usage_logs.attempts` JSON 格式必须保持兼容。
- 管理端日志详情不能破。
- 速率限制 token 回写不能漏。

---

### P14：把 SSE / thinking 迁出 proxy

**目标**：让 `proxy` 不再拥有 stream protocol 处理细节。

建议目标结构二选一：

方案 A：放到 relay 下：

```text
src/relay/sse.rs
src/relay/thinking.rs
```

方案 B：利用已有 `src/stream/`：

```text
src/stream/sse.rs
src/stream/thinking.rs
```

建议优先方案 A，原因：当前 SSE 处理紧贴 relay stream 执行生命周期。

迁移表：

| 当前 | 目标 |
|---|---|
| `src/proxy/sse.rs` | `src/relay/sse.rs` |
| `src/proxy/thinking_normalizer.rs` | `src/relay/thinking.rs` |
| stream loop 中 conversion/passthrough 分支 | `src/relay/stream.rs` 内部 helper |

验收：

```bash
cargo test sse_integration
cargo test protocol_integration
make check
```

风险：

- SSE event 边界处理必须保持。
- Anthropic/OpenAI error event 格式不能破。
- thinking signature 修复不能破。

---

### P15：拆分 ProxyState

**目标**：把状态边界从“大一统 ProxyState”拆成更清晰的状态组合。

当前：

```rust
ProxyState {
    stats_recorder,
    model_registry,
    cache,
    rate_limiter,
    queue,
    pool,
    http_client,
    lb_state,
    key_counter,
}
```

候选目标：

```rust
GatewayState {
    pool,
    auth/rate/budget dependencies,
    relay_state,
}

RelayState {
    pool,
    http_client,
    cache,
    model_registry,
    stats_recorder,
    scheduler_state,
    key_counter,
}

SchedulerState {
    load_balancer,
    capacity,
    circuit,
    runtime,
    metrics,
    sticky,
}
```

P15 不建议过早开始。应在 P12-P14 后再做，否则会被旧 `proxy/execute.rs` 依赖拖住。

验收：

```bash
make check
rg "ProxyState" src/proxy src/relay src/scheduler src/observability
```

风险：

- 大量 handler 构造状态会受影响。
- 测试初始化 helper 可能需要统一更新。

---

### P16：proxy 退化为 gateway adapter

**目标**：让 `src/proxy` 只保留入口/兼容/error formatting，或者改名为 `gateway`。

最终形态：

```text
src/proxy/ 或 src/gateway/
├── mod.rs             # handle_proxy_request
├── error.rs           # gateway/proxy API error formatting（注意不要建全局 error.rs）
├── auth.rs            # API key / budget / rate limit adapter（可选）
└── response.rs        # OpenAI/Anthropic native error response formatting（可选）
```

不再包含：

- 上游 HTTP 执行
- SSE loop
- scheduler scoring
- runtime feedback
- stats recording
- protocol conversion

验收：

```bash
find src/proxy -maxdepth 2 -type f -print
make check
```

---

## 5. 目标模块归属总表

| 职责 | 当前 | 目标 |
|---|---|---|
| 外部 `/v1/*` 入口 | `proxy/mod.rs` | `proxy/mod.rs` 或 `gateway/mod.rs` |
| error response format | `proxy/mod.rs` | gateway/proxy adapter |
| 非流式上游执行 | `proxy/execute.rs` | `relay/http.rs` |
| 流式上游执行 | `proxy/execute.rs` | `relay/stream.rs` |
| Relay executor adapter | `proxy/relay_executor.rs` | `relay/executor.rs` |
| Stream executor adapter | `proxy/stream_relay_executor.rs` | `relay/stream_executor.rs` |
| SSE utils | `proxy/sse.rs` | `relay/sse.rs` |
| thinking normalizer | `proxy/thinking_normalizer.rs` | `relay/thinking.rs` |
| request log glue | `proxy/execute.rs` | `observability/recorder.rs` |
| usage extraction | `proxy/prepare.rs` / `proxy/sse.rs` / `proxy/execute.rs` | `observability/usage.rs` |
| scoring | `scheduler/scoring.rs` + `proxy/mod.rs::score_candidates` | `scheduler/scoring.rs` + scheduler adapter |
| runtime feedback | `scheduler/runtime.rs` + `proxy/state.rs` | `scheduler/state.rs` or `scheduler/runtime.rs` owner |
| cache | `proxy/cache.rs` | `channel/cache.rs` or gateway shared |
| rate limit | `proxy/ratelimit.rs` | gateway/auth limits |

---

## 6. 风险控制原则

1. **外部协议不变**
   - `/v1/*` 代理 API 必须保持 OpenAI/Anthropic 原生格式。
   - `/api/v1/admin/*` 继续统一 `{code,message,data}`。

2. **每次只迁一个边界**
   - P12 只迁 executor/HTTP 文件归属。
   - P13 只迁 stats/observability。
   - P14 只迁 stream/SSE/thinking。

3. **先 characterization，再移动代码**
   - 对 SSE、usage、attempt log、error format 这种敏感行为，移动前先确认测试覆盖。

4. **允许短期 bridge，但必须标记**
   - 例如 `relay` 暂时依赖 `ProxyState` 可以接受，但要在 checklist notes 中记录，并在 P15 消除。

5. **不要新建全局 `error.rs`**
   - AGENTS 明确提醒错误类型在各模块内定义。

6. **不要往 `protocol/inbound/` / `protocol/outbound/` 放文件**
   - 这两个目录是空目录待重构，不作为本轮落点。

---

## 7. 当前是否应该 commit

建议：**应该先 commit 当前 P1-P10 + P11 文档状态**。

理由：

1. 当前已经完成一个完整可验证里程碑：

```text
Relay/Scheduler 主路径迁移 + runtime/metrics 反馈闭环 + 旧 selector 删除
```

2. `make check` 已通过，适合建立稳定回滚点。

3. 下一阶段 P12-P16 会涉及文件移动和模块边界重排，风险高。开始前应先有 clean checkpoint。

建议 commit message：

```text
refactor: migrate proxy routing to relay scheduler pipeline
```

如果想更保守，也可以拆成两个 commit：

```text
refactor: migrate proxy relay scheduler pipeline

docs: add proxy decomposition roadmap
```

但由于当前工作树已经包含大量前序改动，除非现在人工精细 stage，否则建议先做一个大里程碑 commit。

---

## 8. 下一步推荐

1. 先 commit 当前状态。
2. 从 P12 开始，逐步把 executor 和 HTTP 执行迁到 `relay/`。
3. 每个 P 阶段保持：

```bash
make check
```

通过后再进入下一阶段。
