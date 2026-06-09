---
doc_type: audit-finding
id: F-04
nature: bug
severity: P1
confidence: medium
suggested_action: cs-issue
---

# 网络错误/超时不参与失败治理与重试，削弱熔断和可用性

## 证据

`src/proxy/execute.rs:460-471` 调用单次请求；外层只有 `ProxyError::UpstreamError { status, body }` 会进入同渠道换 key/排除渠道重试分支。普通 `Err(e)` 在 `src/proxy/execute.rs:524-535` 记录后直接返回。

单次请求中，`src/proxy/execute.rs:288-294` 对 `.send().await` 的网络错误映射为 `ProxyError::RequestError`，该错误不会触发 `record_failure` 或排除重试。

## 为什么是问题

稳定性故障常见形式不是 HTTP 5xx，而是 DNS、连接失败、TLS、连接池、上游超时。当前这些路径不会更新失败计数/熔断器，也不会尝试其它渠道。

## 影响

- 一个上游网络不可达时，请求直接失败，不会快速 failover。
- 熔断器对网络类故障学习不到状态，后续请求仍可能持续打故障渠道。
- 统计里有错误，但负载均衡健康状态未同步。

## 建议

将可重试的 `RequestError`/timeout 纳入失败治理：记录 channel failure、排除当前 channel 后继续重试；同时区分请求体构造/协议转换错误，这类不应盲目重试。
