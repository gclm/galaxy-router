# 验收报告:per-key 熔断(阶段 1)

> 多 key 重试健壮性改造 · 阶段 1。承接阶段 0。

## 实施(按 design.md checklist,一步一验证)

### step 1 state.rs 熔断职责分离
- `is_channel_available`:改查 `ChannelStatus::is_available`(黑名单),不再查 `circuit_breaker[default]`
- `record_success_with_ttft` / `record_failure`:移除 `circuit_breaker[default]` 调用(per-key 改由 executor 直接调 pub 字段)
- 验证:scheduler 32 项测试过

### step 2 executor.rs per-key 熔断
- execute key 循环:遍历前 `is_tripped` 检查,熔断 key 跳过
- KeyRetryable 失败:per-key `record_failure`
- `execute_proxy_request` 成功:per-key `record_success`
- 验证:executor 2 项测试过(含 mock upstream 端到端)

### step 3 stream_executor.rs per-key
- `execute_stream` key 循环:同 step 2
- 流回调:`record_success(&sc_upstream_key_hint)`(借用,在 L728 move 之前)
- 验证:relay 52 项测试过

### step 4 新增测试
- `test_circuit_per_key_isolation_within_channel`:同 channel 不同 key 独立熔断
- `is_channel_available_reflects_blacklist`:`is_channel_available` 走黑名单
- 验证:scheduler 34 项测试过

### step 5 全量验证
- `cargo test`:**710 项全过,0 failed**
- `cargo clippy`:**零新 warning**(既有 warning 在 prepare/ratelimit/candidates,非本次)
- `make check` 通过

## 行为变更(已实现,design approved)

| 项 | 改前 | 改后 |
|---|---|---|
| 单 key 失败 | 累计 `channel:default`,5 次熔断**整 channel** | 只熔断该 key,executor 跳过它 |
| `is_channel_available` | 查 `circuit_breaker[default]`(5 次/指数退避) | 查 ChannelStatus 黑名单(3 次/10min) |
| 健康探测失败 | 污染 `circuit_breaker[default]` | 不再污染(顺带修正) |

## 偏离 design(记录)

- design 原写「`record_success`/`record_failure` 加 key_hint 参数」,实施改为「executor 直接调 `lb_state.circuit_breaker`(pub 字段)做 per-key」。理由:LoadBalancerState 签名不变,各 step 可独立编译/验证,改动更局部。design 核心意图(职责分离:circuit_breaker per-key + 黑名单 channel 级)完全保留。

## 顺手发现(不本次修)

- clippy 既有 warning(`prepare.rs:309/313`、`ratelimit.rs:177`、`candidates.rs:167/205/236`),非本次引入。建议后续单独 refactor 清理。
