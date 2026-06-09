# 代理转换链路与调度监控重构方案

> 日期：2026-06-08  
> 状态：draft  
> 参考：Octopus `/Users/gclm/workspace/refs/octopus/`，Sub2API `/Users/gclm/workspace/lab/ai/sub2api/`

## 目标

Galaxy Router 的定位是：**Octopus 式简洁协议代理与管理体验 + Sub2API 式监控统计和自动负载均衡**。

本次重构目标：

1. 收敛当前多处散落的转换链路，形成单一 `RelayPipeline`。
2. 将渠道选择、重试、熔断、粘性会话、并发准入统一到 `Scheduler`。
3. 将统计从“请求结束记录”升级为“每次尝试可审计 + 运行时反馈调度”。
4. 保持管理 API 与代理 API 的现有外部协议不破坏。

## 现状判断

### 当前优点

- 已有 `protocol::{inbound,outbound}`，具备协议互转雏形。
- 已有 `ProxyState`、`ProxyCache`、`LoadBalancerState`、`CircuitBreaker`、`StatsRecorder`。
- 已有渠道分组、模型映射、请求日志、token/cost 统计。
- 已支持 OpenAI Chat / Responses / Anthropic，多协议基础可用。

### 当前主要不足

| 问题 | 表现 | 影响 |
|---|---|---|
| 转换链路散落 | `prepare.rs` 做请求转换，`execute.rs` 做非流式/流式响应转换，SSE usage/错误/think 标签在执行层混杂 | 新增协议会继续复制分支，容易出现多条转换链路 |
| 调度与执行耦合 | `proxy/mod.rs` 选渠道，`execute.rs` 负责失败记录和重试，`state.rs` 负责部分熔断 | 失败治理不完整，网络错误/超时不易统一处理 |
| 并发限制不硬 | `max_concurrency` 主要影响选择，不是 permit 准入 | 高峰仍可打爆单渠道 |
| 尝试记录不完整 | 当前 logs 有 attempts，但 skip、circuit-break、no-key、incompatible 这类决策链不完整 | 排障时看不到“为什么没选某渠道” |
| 统计未反哺调度 | 有请求统计，但调度主要基于活跃数/错误率本地状态 | 无法形成 Sub2API 风格的自动负载均衡闭环 |
| 缓存一致性不足 | 渠道 update、分组 add_item 等已有遗漏 | 管理配置修改后代理表现滞后 |

## 参考项目取舍

### 从 Octopus 学什么

Octopus 的核心优点是**链路简洁且对象边界清晰**：

```text
Handler
  -> parseRequest(inbound)
  -> GroupGetEnabledMap
  -> balancer.Iterator
  -> prepareAttempt
  -> pipeline.Process(inbound -> middleware -> outbound)
  -> RelayMetrics.Save(attempts)
```

建议吸收：

- `relayRun` / `relayAttempt` 的生命周期拆分。
- `Iterator` 统一承载候选排序、粘性优先、skip/circuit/attempt 记录。
- 每次 attempt 有 `StartAttempt -> End`，最终 metrics 保存完整 attempts。
- 转换器选择集中在 `newInbound/newOutbound`，而不是散在 handler/execute 多处。

不建议照搬：

- Go/Gin/axonhub pipeline 本身不可直接迁移。
- Octopus 的统计是轻量聚合，缺少 Sub2API 那种调度评分与监控闭环。

### 从 Sub2API 学什么

Sub2API 的核心优点是**调度可观测 + 自动负载均衡闭环**：

- 粘性层：`previous_response_id`、`session_hash`。
- 负载层：候选账号批量读取 load，硬 acquire slot。
- 评分层：priority、load、queue、error_rate、TTFT 权重打分。
- top-K + 加权随机，避免单一最高分长期垄断。
- runtime EWMA：成功/失败、首 token 时间反哺调度。
- scheduler metrics：命中率、切换率、调度耗时、load skew。
- channel monitor：定期探测、历史明细、日聚合。

建议吸收：

- per-channel concurrency permit，选择后必须 acquire。
- runtime EWMA：error rate、TTFT、latency。
- top-K weighted selection：先筛候选，再加权随机。
- `SchedulerDecision` 持久化/记录，用于排障和看板。
- channel monitor + daily rollup。

不建议照搬：

- Sub2API 面向账号池和多租户，Galaxy Router 是单机 SQLite、小团队场景，应简化。
- 不引入 Redis 作为必需依赖；先用内存 semaphore，后续可扩展。

## 目标架构

### 模块结构建议

```text
src/
├── relay/
│   ├── mod.rs              # handle_relay_request 对外入口
│   ├── run.rs              # RelayRun：一次客户端请求生命周期
│   ├── attempt.rs          # RelayAttempt：一次上游尝试生命周期
│   ├── pipeline.rs         # 请求/响应/流式处理统一 pipeline
│   ├── codecs.rs           # inbound/outbound registry
│   ├── decision.rs         # RelayDecision / AttemptTrace
│   └── error.rs            # relay 内部错误；不新建全局 error.rs
│
├── scheduler/
│   ├── mod.rs              # Scheduler trait + DefaultScheduler
│   ├── iterator.rs         # 候选迭代器，记录 skip/attempt
│   ├── policy.rs           # round-robin/random/failover/weighted/top-k
│   ├── scoring.rs          # priority/load/queue/error/ttft score
│   ├── capacity.rs         # per-channel permit / active load
│   ├── circuit.rs          # 三态熔断，可复用现有 proxy/circuit.rs
│   ├── sticky.rs           # session_hash / previous_response_id
│   └── metrics.rs          # scheduler runtime metrics snapshot
│
├── protocol/
│   ├── model.rs            # CanonicalRequest / CanonicalResponse / StreamEvent
│   ├── codec.rs            # ProtocolCodec trait
│   ├── openai_chat.rs
│   ├── openai_responses.rs
│   └── anthropic.rs
│
├── observability/
│   ├── recorder.rs         # 原 stats::recorder 迁移/增强
│   ├── usage.rs            # usage/cost/token 统一提取
│   ├── monitor.rs          # channel monitor runner
│   └── rollup.rs           # hourly/daily 聚合
│
└── proxy/                  # 过渡期保留薄封装，最终只转调 relay
```

> 注意：AGENTS 里说 `scheduler/` 已合并到 `proxy/scheduler.rs`，不要新建独立目录。这是当前硬约束。若要采用上面结构，必须先通过 `cs-decide` 更新架构决策；否则先放在 `src/proxy/scheduler/*` 或 `src/relay/*`，不直接违反现约束。

## 核心设计

### 1. 单一转换链路：ProtocolCodec

当前是：

```text
client json
  -> Inbound.transform_request
  -> LlmRequest
  -> Outbound.transform_request
  -> upstream json

upstream response
  -> Outbound.transform_response / transform_stream_event
  -> LlmResponse / LlmStreamResponse
  -> Inbound.transform_response / stream_converter
  -> client response
```

问题不是方向错，而是执行位置散落。建议保留 canonical model，但收敛入口：

```rust
trait ProtocolCodec {
    fn endpoint_type(&self) -> EndpointType;
    async fn decode_request(&self, body: Bytes, headers: &HeaderMap) -> Result<CanonicalRequest>;
    fn encode_request(&self, req: &CanonicalRequest, auth: UpstreamAuth) -> Result<UpstreamHttpRequest>;
    async fn decode_response(&self, resp: UpstreamHttpResponse) -> Result<CanonicalResponse>;
    fn encode_response(&self, resp: CanonicalResponse) -> Result<ClientHttpResponse>;
    fn stream_decoder(&self) -> Box<dyn StreamDecoder>;
    fn stream_encoder(&self) -> Box<dyn StreamEncoder>;
}
```

然后 `RelayPipeline` 只做一种流程：

```text
入站协议 codec decode
  -> CanonicalRequest
  -> apply model mapping / param override / normalization
  -> 出站协议 codec encode
  -> HTTP execute
  -> 出站 codec decode
  -> 入站 codec encode
```

如果 client endpoint == upstream endpoint，可以走 fast path，但也必须由同一个 pipeline 判定，不再分散到 `prepare.rs` / `execute.rs` 多处。

### 2. RelayRun / RelayAttempt 生命周期

建议仿 Octopus：

```text
RelayRun::new
  - 解析客户端协议
  - 校验 API Key / model access / budget / rate limit
  - 加载 group
  - 创建 SchedulerIterator

RelayRun::run
  loop iterator.next():
    attempt = prepare_attempt()
    result = attempt.run()
    if success: save metrics + return
    if response already written: save metrics + return
  all failed -> save metrics + protocol-native error
```

`RelayAttempt` 负责：

- 获取 channel + upstream key。
- 检查协议兼容性。
- 检查熔断。
- acquire channel slot。
- 调用 pipeline。
- 结束时 release slot、记录 success/failure、更新 EWMA。

### 3. Scheduler：三层选择 + top-K 加权

调度顺序建议：

```text
Layer 1: previous_response_id / conversation sticky（可选，Responses API 优先）
Layer 2: session_hash sticky
Layer 3: load balance
```

负载均衡层：

```text
候选 = group_items + channel runtime state
过滤：enabled / endpoint compatible / model compatible / not excluded / not circuit-open
打分：
  priority_factor   优先级
  load_factor       active / max_concurrency
  queue_factor      waiting count
  error_factor      EWMA error rate
  latency_factor    EWMA latency 或 TTFT
  health_factor     monitor 最近状态
score = Σ weight_i * factor_i

选择：rank top-K -> weighted random order -> 逐个 try_acquire
```

默认权重建议：

```json
{
  "priority": 1.0,
  "load": 1.2,
  "queue": 0.8,
  "error_rate": 1.5,
  "latency": 0.6,
  "health": 1.0
}
```

### 4. 并发控制：从 active counter 升级为 permit

当前 `active_requests` 只能统计，不能阻止超限。目标：

```text
try_acquire(channel_id, max_concurrency) -> Option<ChannelPermit>
```

- 非流式：attempt 结束 drop permit。
- 流式：stream 结束或客户端断开时 drop permit。
- 无限制：返回 no-op permit。
- 已满：Scheduler 尝试下一个候选；都满时返回 WaitPlan 或 429/503。

单机 SQLite 场景先用内存 semaphore；后续多实例再设计分布式 slot。

### 5. 熔断粒度

建议从当前 `channel_id:default` 升级为：

```text
channel_id : upstream_key_hint : upstream_model : endpoint_type
```

原因：

- 同一渠道多个 key，一个 key 429 不应熔断整个渠道。
- 同一 provider 某模型故障，不代表所有模型故障。
- Chat / Responses / Anthropic endpoint 故障面不同。

### 6. 尝试链路可审计

新增/扩展 attempt trace：

```rust
AttemptTrace {
  attempt_no,
  channel_id,
  channel_name,
  upstream_key_hint,
  requested_model,
  upstream_model,
  client_endpoint,
  upstream_endpoint,
  status: skipped | circuit_break | acquired | success | failed,
  reason,
  duration_ms,
  queue_wait_ms,
  sticky,
  score_snapshot,
}
```

这可以解决“为什么这个请求没走 A 渠道”的排障问题。

### 7. 监控统计

建议分三层：

#### 请求日志明细

保留当前 `request_logs` 风格，但补字段：

- requested_model
- upstream_model
- model_mapping_chain
- client_endpoint
- upstream_endpoint
- scheduler_layer
- scheduler_score
- channel_attempts JSON
- ttft_ms
- queue_wait_ms
- upstream_key_hint

#### Runtime metrics

内存态，用于调度：

- success/failure EWMA
- latency/TTFT EWMA
- active/current waiting
- last_used_at
- circuit state

#### 聚合看板

SQLite 小项目可以先做查询聚合，数据量大后再加 hourly/daily rollup：

- total requests / success rate / error rate
- per channel latency / ttft / cost / tokens
- scheduler sticky hit ratio
- channel switch rate
- load skew
- top failed channels/models

### 8. Channel Monitor

参考 Sub2API，做轻量版即可：

表：

- `channel_monitors`
- `channel_monitor_histories`
- `channel_monitor_daily_rollups`

运行逻辑：

```text
定时扫描 due monitors
  -> 构造 provider-specific probe request
  -> 请求上游
  -> 记录 latency/status/error/model
  -> 更新 channel runtime health
  -> 可选：连续失败自动降低 score 或临时熔断
```

不要一开始做复杂模板系统；先支持每渠道主模型 + 自定义 prompt + interval。

## 实施清单

### Phase 0：先补稳定性地基

- [ ] 修复渠道 update 后未失效 `ProxyCache`。
- [ ] 修复 group add_item 后未失效 `ProxyCache`。
- [ ] 网络错误/超时纳入失败治理和重试。
- [ ] 队列测试取消秒级真实等待。

### Phase 1：建立 RelayRun 骨架，不改外部行为

- [ ] 新增 `src/relay/` 或按现约束新增 `src/proxy/relay/`。
- [ ] 引入 `RelayRun` / `RelayAttempt`，先复用现有 `prepare_proxy_request` 与 `execute_proxy_*`。
- [ ] 将 handler 入口逐步改成调用 `relay::handle`，保留原响应格式。
- [ ] 为非流式 Chat 添加 characterization tests。
- [ ] 为流式 Chat 添加 characterization tests。

### Phase 2：统一转换 pipeline

- [ ] 定义 `ProtocolCodec` 或统一 `CodecPair`。
- [ ] 将 `get_inbound/get_outbound` 迁移到 codec registry。
- [ ] 将请求转换从 `prepare.rs` 移入 `RelayPipeline`。
- [ ] 将非流式响应转换从 `execute.rs` 移入 `RelayPipeline`。
- [ ] 将流式 `outbound -> canonical event -> inbound` 收敛到统一 stream pipeline。
- [ ] 保留 passthrough fast path，但由 pipeline 统一选择。

### Phase 3：Scheduler 迭代器与 attempt trace

- [ ] 新增 `SchedulerIterator`，输出有序候选。
- [ ] 将 skip/no-key/incompatible/circuit-break 都记录为 `AttemptTrace`。
- [ ] 将当前 `select_group_item`、`find_channel_by_model` 逻辑迁入 scheduler。
- [ ] 请求日志保存完整 attempts。
- [ ] 管理端日志详情展示 attempts 链路。

### Phase 4：硬并发准入与 top-K 自动负载均衡

- [ ] 新增 `ChannelCapacityManager`，支持 per-channel permit。
- [ ] Scheduler 选择阶段读取 active/waiting/load_rate。
- [ ] 实现 priority/load/queue/error/latency/health scoring。
- [ ] 实现 top-K + weighted random selection order。
- [ ] `ReportResult(success, ttft/latency)` 更新 runtime EWMA。
- [ ] 增加 scheduler metrics API。

### Phase 5：监控与聚合

- [ ] 新增 channel monitor migrations。
- [ ] 实现 monitor runner。
- [ ] 监控结果反哺 runtime health。
- [ ] 增加 channel health / scheduler dashboard。
- [ ] 根据数据量决定是否做 hourly/daily rollup。

### Phase 6：清理旧链路

- [ ] 删除或瘦身 `proxy/prepare.rs` 中转换逻辑。
- [ ] 删除 `execute.rs` 中重复的非流式/流式转换分支。
- [ ] 将 `protocol/inbound.rs` / `outbound.rs` 合并或明确为 codec 子接口。
- [ ] 更新 `.gclm-harness/decisions/architecture/02-module-design.md`。
- [ ] 更新 README / docs 中的架构说明。

## 验收标准

1. 外部代理协议不变：`/v1/chat/completions`、`/v1/responses`、`/v1/messages` 保持原生协议格式。
2. 每个请求只有一条统一 relay pipeline，无多处重复转换分支。
3. 每条请求日志能看到完整 attempts：skip、circuit、failed、success。
4. 渠道 `max_concurrency` 能硬性限制并发，流式断开后能释放。
5. 网络错误、超时、HTTP 5xx 都能进入失败治理；可重试时自动切换候选。
6. Scheduler metrics 可看到 sticky hit ratio、load balance count、switch rate、latency、load skew。
7. Channel monitor 能定时探测并在管理端看到状态。
8. `make test`、关键协议转换测试、流式测试通过。

## 风险与约束

- 这是跨模块架构重划，不适合一次性大改。必须分 phase 做 parallel change。
- 当前 AGENTS 明确说不要新建独立 `scheduler/` 目录；若要新建，需要先更新架构决策。
- SQLite 单机部署不适合照搬 Sub2API 的 Redis/PG 复杂度；先做内存 runtime + SQLite 明细。
- 流式请求最容易出现 permit 泄露、统计丢失、客户端断开处理不完整，必须优先写测试。
