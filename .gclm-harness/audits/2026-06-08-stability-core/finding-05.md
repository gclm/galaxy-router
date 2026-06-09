---
doc_type: audit-finding
id: F-05
nature: performance
severity: P2
confidence: medium
suggested_action: cs-refactor
---

# TPM 限流是事后软检查，突发并发请求可穿透预算

## 证据

`src/proxy/mod.rs:867-871` 请求开始前检查 RPM 和 TPM。

`src/proxy/ratelimit.rs` 文档说明 TPM 是软检查：只看当前窗口已累计 token 数；真正 token 计数在请求完成后通过 `record_tokens` 更新。非流式记录在 `src/proxy/execute.rs:254`，流式记录在 `src/proxy/execute.rs:1133`。

## 为什么是问题

多个大请求并发进入时，它们开始前都看不到彼此未来 token 消耗，因此都可通过 TPM 检查。完成后才发现窗口已超限，无法阻止已经发往上游的成本和压力。

## 影响

- TPM 限制对突发并发流量保护不足。
- 成本预算与上游 TPM 配额可能被短时间打穿。

## 建议

请求开始时基于 prompt 估算 token 做 reservation，完成后按真实 usage 校正；或对未完成请求维护 in-flight token 估算。对流式请求要在结束/断开时释放或修正预留。
