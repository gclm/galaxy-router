# 问题报告:glm-5.2 限流不触发换 key

> 多 key 重试健壮性改造 · 阶段 0(hotfix,借鉴 octopus 设计)

## 现象

brew 部署(v1.0.0)的 galaxy-router,glm-5.2 账号受限时只试 1 个 key 就给客户端 502,不切换同渠道的其他 key。

## 证据(数据库 `usage_logs` + 运行日志三方互证)

- 渠道「智谱 Coding Plan」(`019e812a`)有 **2 个 enabled key**,但 `attempts` 数组只有 **1 个元素**
- 最近 3 天 glm-5.2 失败 **259 次**,全是 **502**,全是**流式**
- 上游错误体:`[1302][您的账户已达到速率限制,请您控制请求频率]`
- 运行日志零条 `key retry / trying next key`,满屏 `渠道 019e812a 被拉黑 10 分钟`

## 复现(逻辑层,可单测)

`classify_upstream(StatusCode::BAD_GATEWAY, "[1302][您的账户已达到速率限制]")` 当前返回 `UpstreamRetryable`(应为 `KeyRetryable`),导致 `is_key_retryable()`=false → 不换 key。

## 根因

- **位置**:`src/error/proxy.rs:137-150` `KEY_NEEDLES`
- **原因**:关键词列表只有英文 `"rate limit"`,缺中文 `"速率限制"` / 智谱错误码 `"1302"`。流式路径 `stream_executor.rs:272-274` 的 SSE 错误提取分支把 status 硬编码为 502,但 body `[1302][速率限制]` 仍在;因中文不命中 `KEY_NEEDLES`,`classify_upstream` 判为 `UpstreamRetryable`(502 server error),不换 key。

## 期望 vs 实际

- 期望:限流(无论 status 是 429 还是 502,body 含限流语义)→ `KeyRetryable` → 换同渠道其他 key
- 实际:中文限流 body 不被识别 → 不换 key → 客户端 502

## 本阶段修复(最小止血,已完成 ✅)

`KEY_NEEDLES` 增加:`"速率限制"`、`"频率限制"`(通用中文限流语义,覆盖智谱 1302 原文)。
SSE 状态码归因(硬编码 502 → 429)留阶段 2。

### 测试覆盖

- 新增回归 `classify_upstream_chinese_rate_limit_is_key_retryable`(`src/error/proxy.rs`):中文限流 body(智谱 1302 原文 / 通用「速率限制」)+ 502 → `KeyRetryable`
- 全量 `cargo test`:**706 项全过,0 回归**

## 范围外(后续阶段,另开 task)

- 阶段 1:`scheduler/state.rs` per-key 熔断(单 key 受限不连累整渠道拉黑)
- 阶段 2:`stream_executor.rs` SSE 分支状态码归因 + 三层重试 + 兜底分支 `is_key_retryable`
