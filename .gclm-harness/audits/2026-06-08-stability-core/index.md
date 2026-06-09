---
doc_type: audit-index
status: active
slug: stability-core
date: 2026-06-08
scope: proxy core, channel/group cache, rate limiting, queue tests
---

# 稳定性核心审计

## 范围

本次审计聚焦当前稳定性最相关的后端路径：

- `src/proxy/`：代理执行、重试、熔断、负载/并发、队列、限流
- `src/api/handlers/admin/channels/`：渠道业务配置变更与缓存一致性
- `src/api/handlers/admin/groups.rs`：分组业务配置变更与缓存一致性

未深入审计前端、备份导入、统计查询 SQL、协议转换完整正确性。

## 总评

项目已有一些稳定性机制雏形：请求队列、RPM/TPM 限流、渠道拉黑、熔断器、流式有界 buffer、请求统计。但当前实现存在几类会直接影响线上稳定性的缺口：

1. **业务配置缓存一致性不完整**：渠道更新、分组项新增后缓存未失效，可能导致用户在管理端更新后代理仍走旧配置。
2. **并发限制偏“调度建议”而非硬限制**：`max_concurrency` 只用于选择低负载渠道，不能阻止超额请求继续进入同一渠道。
3. **失败治理覆盖不完整**：网络错误/超时等非 HTTP 错误路径不会触发渠道失败、熔断或排除重试。
4. **限流语义偏软**：TPM 只在请求完成后记录，突发大请求可并发穿透限制。
5. **测试稳定性有明显隐患**：队列单测在正常情况下会等待 5 秒，拖慢 CI，且容易被误认为 hang。

## 发现矩阵

| 编号 | 标题 | 性质 | 严重度 | 置信度 | 建议动作 |
|---|---|---|---|---|---|
| F-01 | 渠道更新后未失效 ProxyCache，代理可能继续使用旧上游配置 | bug | P1 | high | cs-issue |
| F-02 | 新增分组项后未失效分组缓存，新路由可能不生效 | bug | P1 | high | cs-issue |
| F-03 | `max_concurrency` 没有硬性准入控制，流量高峰仍会打爆单渠道 | performance | P1 | medium | cs-refactor |
| F-04 | 网络错误/超时不参与失败治理与重试，削弱熔断和可用性 | bug | P1 | medium | cs-issue |
| F-05 | TPM 限流是事后软检查，突发并发请求可穿透预算 | performance | P2 | medium | cs-refactor |
| F-06 | 队列单测依赖 5 秒真实超时，CI 稳定性差 | maintainability | P2 | high | cs-refactor |

## 优先级建议

1. **先修 F-01 / F-02**：这是硬约束 #6 的直接偏离，且会造成“管理端改了但代理不生效”的稳定性错觉。
2. **再修 F-04 / F-03**：提升故障转移与高峰抗压能力，是稳定性主线。
3. **随后优化 F-05 / F-06**：增强限流准确性与 CI 可维护性。

