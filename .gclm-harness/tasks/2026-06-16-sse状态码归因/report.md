# 验收报告:SSE 流内错误状态码归因(阶段 2)

> 多 key 重试健壮性改造 · 阶段 2(收尾)。承接阶段 0、阶段 1。

## 实施(issue 快速通道)

### 改动
- `error/proxy.rs`:新增纯函数 `sse_stream_error_status(body) -> StatusCode`(限流/鉴权语义→429,否则 502),内部委托 `is_key_retryable_upstream_error`;移除后者多余的 `#[allow(dead_code)]`(现已使用)
- `stream_executor.rs:272`:SSE 分支 `status: StatusCode::BAD_GATEWAY`(硬编码)→ `status: sse_stream_error_status(&error)`(按 body 归因)

### 测试(TDD)
- RED→GREEN:`sse_stream_error_status_classifies_rate_limit_as_429`(限流体→429,普通错误→502)
- 全量 `make check`:**712 项 0 failed**,clippy 无新 warning

## 效果

阶段 0+1+2 组合后的完整链路:
1. 限流 → 换 key(阶段0)
2. 单 key 连续失败 → 只熔断该 key,executor 跳过(阶段1)
3. SSE 流内限流(上游 2xx+体内错误,如智谱 1302)→ 归因 429 → **不触发 channel 黑名单** + 客户端收到 429(阶段2)

## 范围外(评估后不做)

- **B 同 key 按 Retry-After 退避**:换 key 已是首选;全 key 限流返回 429+Retry-After 给客户端 SDK 退避更合理(不占服务端连接)
- **C 兜底分支 is_key_retryable**:兜底接的是 `RequestError`(网络错误),换渠道比换 key 合理,`run.rs` 已换渠道

## 多 key 重试健壮性改造 · 总结

| 阶段 | 改动 | commit |
|---|---|---|
| 0 | `KEY_NEEDLES` 补中文限流词 | `f573379` |
| 1 | circuit_breaker per-key + 黑名单 channel 级 | `b27c9a9` |
| 2 | SSE 流内错误状态码归因(502→429) | (本次) |

glm-5.2 账号受限时:换 key → 单 key 熔断隔离 → 不误拉黑 channel,全链路打通。
