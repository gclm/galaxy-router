# Relay + Scheduler TDD 重构设计

> 日期：2026-06-08  
> 状态：approved-by-request  
> 策略：TDD（每个生产代码切片必须先有失败测试）  
> 目标方案来源：`.gclm-harness/solutions/optimizations/relay-scheduler-refactor.md`

## 目标

在不破坏现有外部协议的前提下，把当前散落在 `proxy/prepare.rs`、`proxy/execute.rs`、`proxy/mod.rs`、`proxy/state.rs` 的代理链路重构为：

```text
API handler
  -> RelayRun
  -> SchedulerIterator
  -> RelayAttempt
  -> RelayPipeline
  -> RelayMetrics
```

核心结果：

1. **单一转换链路**：所有协议转换都由 RelayPipeline 管理，避免 prepare/execute 多处重复判断。
2. **统一调度闭环**：选择、跳过、熔断、并发 permit、失败反馈都通过 Scheduler 统一处理。
3. **attempt 可审计**：每个请求能看到完整候选尝试链。
4. **Sub2API 风格自动负载均衡**：priority/load/queue/error/latency/health 打分，top-K + weighted random。
5. **TDD 推进**：每个阶段先写 RED 测试，再写最小实现，最后重构。

## 不变量 / 不能破坏

- `/v1/*` 代理 API 保持上游原生协议格式，不统一包装。
- `/api/v1/admin/*` 管理 API 继续使用 `{code,message,data}`。
- UUID v7 / TEXT ID 规则不变。
- `channels.models` 仍是 JSON 字符串数组。
- 迁移只能新增，不能修改已发布迁移。
- 业务配置变更必须失效对应 `ProxyCache`。
- 当前 AGENTS 约束下，暂不新建根级 `src/scheduler/`；第一阶段先在 `src/proxy/scheduler.rs` 下扩展子模块。若后续要独立目录，先更新架构决策。

## TDD 总纪律

每个切片必须遵循：

```text
RED: 写一个失败测试，确认失败原因是目标行为缺失
GREEN: 写最小生产代码让该测试通过
REFACTOR: 清理命名/结构，保持测试绿色
```

禁止：

- 先写生产代码再补测试。
- 一次写多个行为的测试。
- 为了通过测试改坏外部协议。
- 在重构阶段夹带未测试的新行为。

## 测试分层

| 层级 | 目的 | 文件位置 |
|---|---|---|
| 纯单元测试 | scoring、top-K、attempt trace、capacity permit | `tests/scheduler_*.rs` 或模块内 `#[cfg(test)]` |
| characterization tests | 锁定现有 proxy 行为，防止重构改协议 | `tests/relay_characterization.rs` |
| 集成测试 | 验证真实路由、重试、统计落库 | `tests/relay_integration.rs` |
| 流式测试 | 验证 SSE 转换、TTFT、permit release | `tests/relay_stream.rs` |

## Phase 1：Scheduler 纯逻辑先行

### 目标

先提取不碰 HTTP、不碰 DB 的纯调度逻辑，形成可测试核心。

### TDD 切片

#### 1.1 Scoring factors

RED：给定候选 runtime 状态，计算分数。

行为：

- priority 越小越好。
- load_rate 越低越好。
- waiting_count 越低越好。
- error_rate 越低越好。
- latency/ttft 越低越好。
- health 越健康越好。

GREEN：新增最小 `CandidateScoreInput`、`SchedulerScoreWeights`、`calculate_candidate_score`。

#### 1.2 Top-K ranking

RED：给定多个候选，只返回 score 最高的 K 个，分数相同按 priority/load/waiting/id 稳定排序。

GREEN：实现 `select_top_k_candidates`。

#### 1.3 Weighted order

RED：top-K 生成一个不丢候选、不重复候选的尝试顺序；带 seed 时顺序可复现。

GREEN：实现 deterministic weighted random order。

#### 1.4 Attempt trace

RED：skip、circuit_break、success、failed 都能生成 trace，attempt_no 递增。

GREEN：实现 `SchedulerIteratorTrace` 或 `AttemptTraceBuilder`。

## Phase 2：Capacity permit

### 目标

把 `max_concurrency` 从“负载参考”升级为硬准入。

### TDD 切片

#### 2.1 max_concurrency=0 不限流

RED：无限制渠道多次 acquire 都成功。

GREEN：实现 no-op permit。

#### 2.2 max_concurrency=N 硬限制

RED：N 个 permit 后第 N+1 个 try_acquire 失败。

GREEN：实现 per-channel semaphore / atomic slot。

#### 2.3 permit drop 释放容量

RED：drop permit 后可再次 acquire。

GREEN：实现 RAII release。

#### 2.4 流式释放保护

RED：模拟 stream drop 后 permit 被释放。

GREEN：将 release guard 接入流式结束路径。

## Phase 3：RelayRun / RelayAttempt 骨架

### 目标

先建立 Octopus 风格生命周期，但暂时复用现有执行函数。

### TDD 切片

#### 3.1 RelayRun records success attempt

RED：成功请求产生一个 success attempt trace。

GREEN：实现 `RelayRun` 最小骨架，调用 fake attempt executor。

#### 3.2 RelayRun retries next candidate when attempt fails before response written

RED：第一个 candidate 失败且未写响应，第二个 candidate 成功。

GREEN：循环 iterator。

#### 3.3 RelayRun stops when response already written

RED：流式/部分响应已写出时，不再重试。

GREEN：增加 `response_written` 语义。

#### 3.4 RelayMetrics saves attempts even on all failed

RED：所有 candidate 失败仍保存 attempts。

GREEN：接入 recorder trait。

## Phase 4：RelayPipeline 统一转换链路

### 目标

把请求转换、响应转换、流式转换从 `prepare.rs` / `execute.rs` 收敛到一个 pipeline。

### TDD 切片

#### 4.1 Passthrough fast path preserves body and native auth

RED：OpenAI Chat -> OpenAI Chat 时请求 body 基本保持，鉴权头正确。

GREEN：pipeline passthrough。

#### 4.2 Cross protocol request uses canonical model exactly once

RED：Anthropic -> OpenAI Chat 只经过 inbound decode + outbound encode 一次。

GREEN：pipeline conversion path。

#### 4.3 Non-stream response converts by reverse codec pair

RED：OpenAI Chat 上游响应能转回 Anthropic 格式。

GREEN：pipeline response encode。

#### 4.4 Stream response converts by event pipeline

RED：SSE chunk 经 outbound stream decoder -> canonical event -> inbound stream encoder。

GREEN：统一 stream pipeline。

## Phase 5：Scheduler 接入真实 proxy

### 目标

用 Scheduler 替换当前 `select_group_item` / `find_channel_by_model` 的核心路径。

### TDD 切片

#### 5.1 session sticky 优先但必须可用

RED：sticky channel 可用时优先；熔断/排除时跳过。

GREEN：sticky layer。

#### 5.2 circuit-open candidate emits circuit_break trace

RED：熔断渠道不尝试上游，但记录 trace。

GREEN：iterator skip circuit。

#### 5.3 capacity full tries next candidate

RED：第一渠道满载，选择第二渠道。

GREEN：capacity manager 接入。

#### 5.4 network error triggers retry and failure feedback

RED：第一个渠道连接错误，第二渠道成功，第一渠道 runtime failure 增加。

GREEN：错误分类与 retry。

## Phase 6：监控统计反哺调度

### 目标

接入 runtime EWMA、scheduler metrics、monitor health。

### TDD 切片

#### 6.1 report success lowers error EWMA

RED：失败后成功，error_rate 下降但不归零。

GREEN：EWMA runtime stats。

#### 6.2 report TTFT updates latency factor

RED：低 TTFT 候选 score 高于高 TTFT 候选。

GREEN：TTFT EWMA 接入 scoring。

#### 6.3 scheduler metrics count sticky/loadbalance/switch

RED：不同 layer 选择更新不同计数。

GREEN：metrics snapshot。

#### 6.4 monitor failure lowers health factor

RED：monitor 最近失败导致 score 降低或不可用。

GREEN：health factor。

## Phase 7：清理旧链路

### 目标

在 characterization tests 保护下，逐步删除旧分支。

### TDD 切片

- 删除前跑完整 tests。
- 每删除一段旧逻辑，先写“无旧入口仍保持行为”的 characterization test。
- 删除后跑完整 tests。

## 验收标准

- [ ] `cargo test` 全绿。
- [ ] `make check` 通过。
- [ ] Chat/Responses/Anthropic 非流式原生响应格式不变。
- [ ] Chat/Responses/Anthropic 流式响应格式不变。
- [ ] 每次请求日志含完整 attempts。
- [ ] `max_concurrency` 是硬限制。
- [ ] 网络错误/超时可重试并反馈调度。
- [ ] Scheduler metrics 有 sticky hit、load balance select、switch、latency、load skew。
- [ ] 转换逻辑只有 RelayPipeline 一个主入口。

## 第一批实施顺序

1. `scheduler_scoring`：纯单元测试，风险最低。
2. `scheduler_capacity`：硬并发 permit。
3. `attempt_trace`：可审计尝试链。
4. `relay_run_fake_executor`：建立 RelayRun 生命周期。
5. `relay_pipeline_passthrough`：统一 pipeline 的最小闭环。
6. `relay_pipeline_conversion`：跨协议转换闭环。

