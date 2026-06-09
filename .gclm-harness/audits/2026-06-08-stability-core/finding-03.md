---
doc_type: audit-finding
id: F-03
nature: performance
severity: P1
confidence: medium
suggested_action: cs-refactor
---

# `max_concurrency` 没有硬性准入控制，流量高峰仍会打爆单渠道

## 证据

`src/proxy/mod.rs:350-367` 只判断是否启用并发配置并切换选择策略；`src/proxy/mod.rs:392-407` 按最低负载率过滤，但没有排除已达到 `max_concurrency` 的渠道。

`src/proxy/execute.rs:274-275` 和 `src/proxy/execute.rs:598-599` 在选择完成后直接增加活跃请求：

```rust
ensure_channel_status(...).await;
increment_active(&channel_id).await;
```

没有原子“检查 active < max 再占位”的准入步骤。

## 为什么是问题

当前 `max_concurrency` 更像负载均衡评分参数，不是容量保护。并发请求同时选路时都可能看到相近 active 值，并全部进入同一渠道，导致超过上游并发上限。

## 影响

- 高峰期请求会集中打到同一个低延迟/高权重渠道。
- 上游 429/5xx 增多，进一步触发重试风暴。
- 活跃请求计数只能事后观测，不能保护上游。

## 建议

引入 per-channel semaphore 或原子 `try_acquire_channel_slot`，选择阶段排除已满渠道，执行阶段持有 permit，流式请求在 stream drop/结束时释放。该优化应覆盖非流式和流式。
