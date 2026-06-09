# Proxy 退场完整路径图

> 日期：2026-06-09  
> 状态：ready-for-iteration  
> 关联清单：`proxy-retirement-checklist.yaml`  
> 上游方案：`relay-scheduler-tdd-refactor-design.md`、`relay-scheduler-refactor.md`、`proxy-relay-migration-checklist.yaml`  
> 当前目标：在保持 `/v1/*` 原生协议兼容和 `make check` 绿色的前提下，把 `src/proxy` 从核心实现模块降级为薄 gateway adapter。

---

## 1. 结论先行

当前重构不是停滞，而是处于 **主路径迁移完成、模块归位未完成** 的中间态。

已经完成：

```text
请求调度生命周期：proxy 内部 retry loop -> relay::RelayRun / RelayStreamRun
调度算法核心：proxy selector -> scheduler::{scoring, capacity, trace, runtime, metrics}
协议转换核心：proxy 分散转换 -> relay::pipeline + protocol/*
生产反馈闭环：成功/失败/TTFT/monitor -> ChannelRuntimeManager / SchedulerMetrics
旧 selector：已删除 select_channel_for_proxy/select_group_item/find_channel_by_model 等
```

尚未完成：

```text
HTTP 单次执行仍在 src/proxy/execute.rs
SSE/thinking 处理仍在 src/proxy/sse.rs / thinking_normalizer.rs
stats/usage/request log glue 仍在 src/proxy/execute.rs
ProxyState 仍是大一统状态聚合
src/proxy 仍承担 gateway + relay + observability 的混合职责
```

因此接下来不应该继续增加新能力，而应该进入 **P12-P16 模块退场迭代**。

---

## 2. 不可破坏边界

这些约束在后续每个阶段都必须保持：

1. `/v1/*` 代理 API 保持 OpenAI/Anthropic 原生协议格式，不做 `{code,message,data}` 包装。
2. `/api/v1/admin/*` 继续使用统一 JSON 格式 `{code,message,data}`。
3. 不往 `src/protocol/inbound/` 和 `src/protocol/outbound/` 放文件。
4. 不新建全局 `error.rs`，错误类型保留在模块内。
5. channel/group/api key/model 配置变更后继续正确失效 `ProxyCache`。
6. 每个阶段结束必须至少跑 `make check`。
7. SSE、usage、attempt log、错误响应格式属于高风险行为，移动前后必须用 characterization 或集成测试保护。

---

## 3. 当前真实调用路径

### 3.1 非流式

```text
handle_proxy_request
  -> proxy_request
  -> build_relay_candidates
  -> RelayRun
  -> ProxyRelayExecutor
  -> execute_proxy_request
  -> RelayPipeline
  -> native upstream/protocol response
```

### 3.2 流式

```text
handle_proxy_request
  -> proxy_stream
  -> build_relay_candidates
  -> RelayStreamRun
  -> ProxyStreamRelayExecutor
  -> execute_proxy_stream
  -> RelayPipeline stream conversion
  -> native SSE stream
```

### 3.3 当前问题

`RelayRun` 和 scheduler 已经是主路径，但真实 executor、HTTP/SSE 执行、记录落库仍挂在 `src/proxy` 下，所以源码结构和运行时职责不完全一致。

---

## 4. 目标终态

目标不是立刻删除 `src/proxy`，而是让它只剩 gateway adapter。

```text
src/proxy/ 或未来 src/gateway/
├── mod.rs            # /v1/* handler adapter
├── error.rs          # 仅 proxy/gateway native error formatter，可选；不要建全局 error.rs
├── auth.rs           # API key / budget / rate-limit glue，可选
└── response.rs       # OpenAI/Anthropic native error response helper，可选
```

最终不应再由 `src/proxy` 拥有：

- 上游 HTTP 执行
- SSE loop / stream conversion 细节
- scheduler scoring/capacity/runtime/metrics
- request log / usage / cost 落库 glue
- thinking tag normalizer
- 大一统 runtime state

---

## 5. 模块归属目标表

| 职责 | 当前位置 | 目标位置 | 迁移阶段 |
|---|---|---|---|
| `/v1/*` route adapter | `src/proxy/mod.rs` | `src/proxy/mod.rs` 或 `src/gateway/mod.rs` | P16 |
| proxy native error formatting | `src/proxy/mod.rs` | gateway/proxy 内部 | P16 |
| 非流式真实 executor | `src/proxy/relay_executor.rs` | `src/relay/executor.rs` | P12 |
| 流式真实 executor | `src/proxy/stream_relay_executor.rs` | `src/relay/stream_executor.rs` | P12 |
| 单次非流式 HTTP 请求 | `src/proxy/execute.rs::execute_proxy_request` | `src/relay/http.rs::execute_once` | P12 |
| 单次流式 HTTP/SSE 请求 | `src/proxy/execute.rs::execute_proxy_stream` | `src/relay/stream.rs::execute_once` | P12/P14 |
| Relay lifecycle | `src/relay/run.rs` | 保持 | 已完成 |
| Relay protocol pipeline | `src/relay/pipeline.rs` | 保持 | 已完成 |
| SSE 边界和 event 工具 | `src/proxy/sse.rs` | `src/relay/sse.rs` | P14 |
| thinking normalizer | `src/proxy/thinking_normalizer.rs` | `src/relay/thinking.rs` | P14 |
| AttemptStats | `src/proxy/execute.rs` | `src/observability/attempt.rs` | P13 |
| request log 落库 glue | `src/proxy/execute.rs` | `src/observability/recorder.rs` | P13 |
| usage/cost 汇总 | `src/proxy/execute.rs` / `prepare.rs` / `sse.rs` | `src/observability/usage.rs` / `cost.rs` | P13 |
| scoring/capacity/trace/runtime/metrics | `src/scheduler/*` + 少量 proxy glue | `src/scheduler/*` | P15 |
| `LoadBalancerState` | `src/proxy/state.rs` | `src/scheduler/state.rs` 或 scheduler-owned state | P15 |
| `ProxyState` 聚合 | `src/proxy/mod.rs` | `GatewayState` + `RelayState` + `SchedulerState` | P15 |
| cache | `src/proxy/cache.rs` | `src/channel/cache.rs` 或 shared gateway cache | P15/P16 |
| rate limit | `src/proxy/ratelimit.rs` | gateway/auth limits，可保留或后迁 | P16 |

---

## 6. 分阶段路径

### P12：Executor 与单次 HTTP 执行迁入 relay

目标：先把“真实 attempt 执行”从 proxy 迁到 relay，不改变行为。

建议新增/调整：

```text
src/relay/executor.rs
src/relay/stream_executor.rs
src/relay/http.rs
src/relay/stream.rs
```

迁移顺序：

1. 移动 `ProxyRelayExecutor` 到 `relay::executor`，保留类型别名或 re-export 降低一次性改动。
2. 移动 `ProxyStreamRelayExecutor` 到 `relay::stream_executor`。
3. 将 `execute_proxy_request` 包装/重命名为 `relay::http::execute_once`。
4. 将 `execute_proxy_stream` 包装/重命名为 `relay::stream::execute_once`。
5. 暂时允许 `relay` 依赖 `ProxyState` 作为 bridge，但在 checklist notes 中记录，P15 必须消除。

验收：

```bash
cargo test relay_run --test relay_run_tdd
cargo test sse_integration
make check
```

---

### P13：Observability 从 proxy/execute.rs 拆出

目标：把统计、usage、cost、request log 从 proxy 执行逻辑中拆出。

建议新增：

```text
src/observability/mod.rs
src/observability/attempt.rs
src/observability/recorder.rs
src/observability/usage.rs
src/observability/cost.rs
```

迁移顺序：

1. 移动 `AttemptStats` 和 attempt JSON adapter。
2. 移动 `save_request_record`。
3. 抽出 non-stream completion record helper。
4. 抽出 stream completion/drop record helper。
5. 抽出 usage/cost/token fallback helper。

验收：

```bash
cargo test stats::recorder --lib
cargo test sse_integration
make check
```

---

### P14：SSE / thinking 迁出 proxy

目标：`proxy` 不再拥有 stream protocol 细节。

建议新增/调整：

```text
src/relay/sse.rs
src/relay/thinking.rs
```

迁移顺序：

1. 移动 `proxy::sse` 到 `relay::sse`。
2. 移动 `proxy::thinking_normalizer` 到 `relay::thinking`。
3. 将 stream loop 中 SSE event decode/convert helper 收敛到 `relay::stream`。
4. 清理 proxy 对 SSE/thinking 的 re-export。

验收：

```bash
cargo test sse_integration
cargo test protocol_integration
make check
```

---

### P15：拆分 ProxyState

目标：消除 P12-P14 期间允许的 bridge，把状态所有权归位。

候选结构：

```rust
GatewayState {
    pool,
    rate_limiter,
    queue,
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
    channel_status,
    circuit_breaker,
    capacity,
    sticky,
    runtime,
    metrics,
}
```

迁移顺序：

1. 先引入只读 wrapper，不立刻删除 `ProxyState`。
2. 修改 relay executor 依赖 `RelayState`。
3. 修改 scheduler scoring/feedback 依赖 `SchedulerState`。
4. 修改 handler 只依赖 `GatewayState` 或保留兼容别名。
5. 删除 relay -> proxy 的 bridge 依赖。

验收：

```bash
rg "ProxyState" src/proxy src/relay src/scheduler src/observability
make check
```

---

### P16：Proxy 降级为 gateway adapter

目标：完成 proxy 退场，或为后续 rename `proxy -> gateway` 做准备。

最终检查：

```bash
find src/proxy -maxdepth 2 -type f -print
rg "execute_proxy_request|execute_proxy_stream|AttemptStats|score_candidates|thinking_normalizer|SSE" src/proxy
make check
```

完成标准：

- `src/proxy` 不再包含 HTTP/SSE 执行细节。
- `src/proxy` 不再包含 scheduler 核心算法。
- `src/proxy` 不再包含 request log/usage/cost 落库核心逻辑。
- `src/proxy` 只承担 `/v1/*` adapter、鉴权/限流/budget glue、native error response formatting。

---

## 7. 推荐迭代纪律

每轮只做一个小目标：

```text
RED：补 characterization / 编译暴露移动边界
GREEN：移动最小代码使测试通过
REFACTOR：清理 re-export、命名、可见性
CHECKPOINT：make check 通过后更新 checklist notes
```

每个阶段结束判断是否 commit：

- P12 完成：建议 commit
- P13 完成：建议 commit
- P14 完成：建议 commit
- P15 完成：必须 commit
- P16 完成：必须 commit

---

## 8. 当前 commit 建议

现在建议先 commit 当前状态。

理由：

1. P1-P11 已形成完整稳定 checkpoint。
2. 最近 `make check` 已通过。
3. 下一步 P12 开始会有文件移动和模块边界重排，回滚风险显著增加。
4. 当前改动包含核心代码、测试和架构文档，应该先固化。

建议 commit：

```text
refactor: migrate proxy routing to relay scheduler pipeline
```

如果要拆 commit，建议：

```text
refactor: migrate proxy relay scheduler pipeline
docs: add proxy retirement pathmap
```

但当前 working tree 已包含较多连续改动，若不做精细 stage，一个大里程碑 commit 更实际。
