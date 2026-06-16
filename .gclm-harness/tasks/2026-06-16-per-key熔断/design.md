---
doc_type: design
feature: 2026-06-16-per-key熔断
status: approved
---

# per-key 熔断 设计方案

> 多 key 重试健壮性改造 · 阶段 1。承接阶段 0(已修复「限流不换 key」)。

## 1. 背景与根因

阶段 0 让限流(1302/中文)触发换 key。但遗留:熔断/黑名单**仍是 channel 级**,导致:

- 已知熔断的 key 每次请求仍被重复尝试(浪费一次失败 RTT)
- 两个 key 都限时,`on_attempt_failed` 累计到 channel 级 → 拉黑/熔断整 channel

**根因**:两套容错机制都按 channel 聚合,不分 key:

| 机制 | key 构成 | 现状 |
|---|---|---|
| `CircuitBreaker`(circuit.rs) | `format!("{}:{}", channel_id, key_hint)` | 签名支持 per-key,但 state.rs:226/248/254 硬编码 `"default"` → 所有生产 key 共享 `channel:default` |
| `ChannelStatus` 黑名单(state.rs:239) | channel | `failure_count` channel 级 |

且 executor 的 key 循环(`executor.rs:134` / `stream_executor.rs:789`)**没有 per-key 熔断检查**,遍历 key 直接发请求。

**隐藏 bug**(顺带修正):健康探测 `scheduler_task.rs:139` 调 `lb_state.record_failure` → 内部 `circuit_breaker.record_failure(channel, "default")`(state.rs:248)→ **健康探测失败污染生产 `default` 熔断器**,5 次后整 channel 被生产请求跳过。

## 2. 目标

1. 单 key 连续失败 → 只熔断该 key,executor 跳过它、用其他 key
2. 单 key 熔断**不影响**同 channel 其他 key,也**不拉黑**整 channel
3. channel 级熔断保留(全 key 都失败时),由 ChannelStatus 黑名单负责
4. 修正健康探测污染 `default` 的问题

## 3. 方案:职责分离

- `CircuitBreaker` → **per-key 快路径**(executor key 循环内 check / record)
- `ChannelStatus` 黑名单 → **channel 级慢路径**(`on_attempt_failed` 记录 + `is_channel_available` 检查)

### 3.1 改动清单

**① `executor.rs::execute` key 循环(L134-183)**
- 遍历 key 前:`if self.state.lb_state.circuit_breaker.is_tripped(channel_id, &key_hint).0 { continue }`(跳过熔断 key)
- key 失败且 `is_key_retryable` 时:`circuit_breaker.record_failure(channel_id, &key_hint).await` 后 `continue`
- 成功:由 `execute_proxy_request` 内 `record_success` 带 key_hint

**② `executor.rs::execute_proxy_request` 成功路径(L290-293)**
- `record_success(channel, latency)` → 加 `key_hint` 参数透传

**③ `stream_executor.rs::execute_stream` key 循环(L789-850)+ 成功/失败回调(L660-670)**
- key 循环加 `is_tripped` 检查 + per-key `record_failure`
- 流结束回调 `record_success_with_ttft` 带 key_hint(**⚠️ key_hint 需 `move` 进异步 stream 回调闭包**,实施时验证所有权)

**④ `state.rs`**
- `record_success(channel, latency)` / `record_success_with_ttft(channel, latency, ttft)`:加 `key_hint` 参数,`circuit_breaker` 用真实 key_hint(原 `"default"`)
- `record_failure(channel, should_blacklist)`:**移除** `circuit_breaker.record_failure` 调用(circuit_breaker 改由 executor per-key 管;`on_attempt_failed` 是 channel 级无 key_hint,不该碰 per-key 熔断)。只保留 ChannelStatus + runtime 更新
- `is_channel_available(channel)`:**改查 `ChannelStatus::is_available`(黑名单)**,不再查 `circuit_breaker["default"]`(per-key 化后 `"default"` 不再被更新)

**⑤ 调用方同步**
- `executor.rs:112` / `stream_executor.rs:767` `on_attempt_failed` → `record_failure`:签名不变(channel 级)
- `scheduler_task.rs:130/139` 健康探测 `record_success`/`record_failure`:加占位 key_hint(如 `"health"`),或保持 channel 级语义——因 ④ 已移除 record_failure 的 circuit_breaker 调用,健康探测不再污染生产熔断器

### 3.2 执行顺序(checklist)

1. **state.rs**:record 方法加 key_hint + `is_channel_available` 改黑名单 + `record_failure` 去 circuit_breaker → `cargo test -p state` 编译+单测过
2. **executor.rs**:execute key 循环加 `is_tripped` + per-key record;`execute_proxy_request` record_success 带 key_hint → 编译+测试
3. **stream_executor.rs**:同上 + 流回调 key_hint `move` → 编译+测试
4. **新增测试**:单 key 熔断被跳过 / 其他 key 不受影响 / channel 黑名单仍生效
5. 全量 `make test` + `make check`

每步独立验证(一步一做,不批量)。

### 3.3 验证点 / 测试

- 单 key 连续 5 次失败 → `circuit_breaker[channel:key]` Open → executor 跳过该 key(新测试)
- 同 channel 另一个 key 不受影响(新测试)
- channel 全 key 失败 → ChannelStatus 拉黑 → `is_channel_available` false(新测试)
- circuit.rs 现有 per-key 测试(`test_circuit_different_keys`)不破坏
- 全量测试通过

## 4. 行为变更(显式,需 review 确认)

| 项 | 改前 | 改后 |
|---|---|---|
| 单 key 失败 | 累计到 `channel:default`,5 次熔断**整 channel** | 只熔断该 key,其他 key / channel 不受影响 |
| `is_channel_available` | 查 `circuit_breaker[default]`(5 次 / 指数退避 60–600s) | 查 ChannelStatus 黑名单(3 次 / 固定 10min) |
| channel 级熔断阈值 | 5 次失败 | 3 次失败(更易拉黑,但仅全 key 失败时触发) |
| 健康探测失败 | 污染 `circuit_breaker[default]` | 不再污染(修正) |

**核心决策点**:`is_channel_available` 从查熔断器改为查黑名单,channel 级熔断参数变化(5次→3次)。如果你希望保留熔断器的指数退避语义,可改方案 B(见下)。

### 方案 B(备选,未采用)

保留 `circuit_breaker["default"]` 作为 channel 级聚合熔断(`on_attempt_failed` 仍更新它),同时新增 per-key 检查。双层熔断。**未采用原因**:两套 channel 级机制(黑名单 + default 熔断)语义重叠,维护负担大。

## 5. 风险 + 回滚

- 风险:`is_channel_available` 改黑名单后,channel 级熔断更敏感(3 次)。但仅在**全 key 失败**时触发,且黑名单有 10min 自动恢复 + 成功衰减(`record_success` 失败计数减半)
- 风险:stream 回调 `key_hint` `move` 可能引入所有权编译问题,步骤 3 重点验证
- 回滚:git revert(单 commit)

## 6. 范围外(阶段 2)

- SSE 分支状态码归因(502→429)
- 三层重试(同 key 按 Retry-After 退避)
- 兜底分支 `is_key_retryable`
