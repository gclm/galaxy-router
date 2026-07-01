---
doc_type: design
feature: 2026-07-01-渠道测试交互重构
status: approved
complexity: complex
---

# 渠道配置简化：删除思维链处理 + 测试交互重构

> **范围扩大（2026-07-01 更新）**：原计划只重构测试交互 + 砍 fix_signature 保留 extract_tags。
> 调研 axonhub（Go AI gateway）后发现它**完全不做 `<think>` 标签抽取 / signature 修复**——
> 它面向标准 provider，reasoning 走核心模型标准字段（ReasoningContent / ReasoningSignature），不碰 content。
> galaxy 当前对接的上游（智谱 GLM / 商汤 / 京东 / MiniMax）暂无「把 thinking 塞 content」的真实需求，
> 两个思维链开关线上无人启用。决定 **extract_tags + fix_signature 全删**，等有真实非标准上游再重新设计。
>
> 本 design 合并两块：① 删除思维链**运行时处理**功能 ② 渠道测试重构（连通性 + 思维链诊断）。
>
> **关键区分**：运行时思维链处理（normalizer 应用 + extras 开关 + DB 列）**全删**；但渠道测试**保留
> 思维链检测**作为纯诊断信息——探测 payload 带 thinking 参数触发上游返回，检测结果（是否含思维链 +
> 样本）展示在测试结果区。因运行时开关已删，检测结果只供了解上游特性，**不驱动任何配置**（无「应用」按钮）。

## 1. 背景与根因

### 1.1 思维链功能删除（本轮新增）

galaxy 现有两套思维链处理（端点级 `extras.thinking.{extract_tags, fix_signature}`）：

| 开关 | 作用 | 针对场景 |
|---|---|---|
| extract_tags | 抽 `<think>...</think>` 标签到 reasoning_content | MiniMax-M3 等把 thinking 塞 content 的非标准上游 |
| fix_signature | 修 GLM-5.1 把 signature 放 content_block_start 的问题 | GLM-5.1 anthropic 端点 signature 位置异常 |

**删除理由**：
1. axonhub 调研证实：标准 provider 走字段（reasoning_content / thinking block / signature_delta），不需要标签解析。galaxy 的标签抽取面向的「塞 content」场景当前无真实上游需求。
2. 线上 extras.thinking 为空，两个功能实际无人启用。
3. 保留成本：thinking_normalizer.rs 470+ 行状态机 + stream_executor 双路径 normalizer 挂载 + 端点卡片两个开关。

**删除边界**：只删「运行时处理」（normalizer 应用 + extras 开关 + DB 列）。渠道测试的「思维链检测」**保留**——它是诊断工具（告诉用户上游是否会返回思维链），与运行时处理无关，删运行时处理不影响测试诊断。

### 1.2 渠道测试交互（原有问题，部分随删除自动消失）

| # | 问题 | 根因 | 处理 |
|---|---|---|---|
| ① | 并发测试显示失败但实际成功 | `probe_endpoint_raw` 用 `resp.bytes().await` 等 SSE 连接关闭，keep-alive 超时误判 | **修**：改 bytes_stream 增量读 |
| ② | 思维链检测过度应用 | detect 渠道级合并 + 前端全端点应用 | **消失**：思维链功能删，detect 跟着删 |
| ③ | 思维链检测漏报 | build_detect_payload 不带 thinking 参数 | **消失**：不再检测 thinking |
| ④ | 渠道级测试语义混乱 | ChannelDetail「测试」按钮实际只测单端点 | **修**：删渠道级测试，端点卡片内测试 |

## 2. 目标与约束

**目标**：
1. **删除思维链处理功能**：thinking_normalizer.rs 整文件 + stream_executor normalizer 挂载 + DB thinking_mode 列 + endpoint extras 字段 + 前端 thinking 开关 / detect 区
2. **渠道测试 = 连通性 + 思维链诊断**：合并 test+detect 为端点级 test_endpoint，返回 status/latency/ttft/tokens **+ thinking_detected/sample**（纯诊断，无「应用」）
3. **修并发测试 bug**：probe_endpoint_raw 改 bytes_stream 增量读（遇 `[DONE]` 即停）
4. **端点卡片重构**：删 thinking 开关，测试结果放 Headers 下
5. **删渠道级测试**：ChannelDetail 测试按钮删

**明确不做**：
- 不保留任何 thinking 处理代码（连 extract_tags 一起删，不是「只剩 extract_tags」）
- 不为「未来可能需要的非标准上游」预留扩展位（extras 字段一起删，YAGNI；重新设计时从头加）
- 测试的思维链检测**只展示不应用**（运行时开关删了，检测结果不驱动配置变更）

## 3. 方案

### 3.1 删除思维链功能（后端 + DB）

- 删 `src/protocol/thinking_normalizer.rs` 整文件 + `src/protocol/mod.rs` 的 `pub mod thinking_normalizer;`
- `src/relay/stream_executor.rs`：
  - 删 import（:17）、`thinking_flag` helper（:56-63）
  - 删 normalizer 创建块（:471-485：extract_think_tags/fix_signature/passthrough_normalizer/conversion_extractor）
  - 转换路径删 `conversion_extractor` 调用（:550-552）
  - 直通路径删 `passthrough_normalizer` 调用（:717-729），还原为直接 `stream_send(event_bytes)`
- `src/api/handlers/admin/channels/probe.rs`：`analyze_response` 删 `<think>` + signature 检测分支（:659-691），随 detect 整体删
- **DB 迁移 16** `src/db/migrations/16_drop_channel_thinking_mode.sql`：`ALTER TABLE channels DROP COLUMN thinking_mode;`（dead column，SQL 已不读，迁移 15 漏 DROP）
- **endpoint `extras` 字段删除**：从 `EndpointConfig` struct（types.rs:93-95）+ 前端 type（types.ts:52）删除；清理 `extras: None` 占位（cache.rs:164 / prepare.rs:482 / channel.rs:107,114）。endpoints 是 channels 表 TEXT JSON 列，删 struct 字段后旧 JSON 的 extras 被 serde 忽略，**无需 DB 迁移**

### 3.2 渠道测试：连通性 + 思维链诊断（后端）

- 新接口 `POST /api/v1/admin/channels/{id}/test-endpoint`：
  ```
  入参 TestEndpointRequest: { endpoint_type, model, api_key }
  出参 TestEndpointResponse: {
    success, status, latency_ms, ttft_ms,
    prompt_tokens, completion_tokens, output_content, error?,
    thinking_detected: bool, thinking_sample: String   // 诊断：上游是否返回思维链 + 样本
  }
  ```
- **探测 payload 带 thinking 参数**（触发上游返回思维链，修原漏报）：Anthropic `thinking:{type:"enabled",budget_tokens:1024}`（已有）；OpenAiChat/OpenAiResponse 加 thinking 开启参数（实现时 grep protocol/outbound 确认参数名）
- `probe_endpoint_raw` 改 `resp.bytes_stream()` 逐 chunk 增量读：累计 content + usage + thinking 证据，**遇 `[DONE]` 立即停止**（不等连接关闭），超时保留兜底
- 思维链检测：响应文本含 `<think>` 标签（协议无关字符串匹配）→ `thinking_detected=true` + 截取样本；**只检测不应用**
- 删 `test_channel`（probe.rs:19）+ `detect_channel_quirks`（probe.rs:184）handler + 路由（检测能力并入 test_endpoint，per-endpoint 不合并）
- 删旧 types：`TestChannelRequest/Response`、`DetectRequest/Response/EndpointDetection`、recommendations 字段 → 新 `TestEndpointRequest/Response`

### 3.3 前端

- `ChannelForm.tsx`：删 thinking 开关（extract_tags + fix_signature checkbox，:318-335）+ 思维链检测区（:472-478）+ `getEndpointThinking`/`setEndpointThinking`（:197-208）+ `applyDetectedRecommendations` thinking 部分（:141-151）；端点卡片头部加「测试」按钮，结果区放 Headers 下（连通性一行 + 思维链诊断一行：检测到/样本，**无开关/应用**）
- `ChannelDetail.tsx`：删「测试」按钮（:166）+ onTest props
- `api/types.ts` + `channels.ts`：删 extras/recommendations；加 TestEndpoint* types + testEndpoint API

## 4. 改动清单（挂载点）

| 文件 | 改动 |
|---|---|
| `protocol/thinking_normalizer.rs` | **整文件删** |
| `protocol/mod.rs` | 删 `pub mod thinking_normalizer;` |
| `relay/stream_executor.rs` | 删 normalizer import/thinking_flag/创建/使用（转换+直通路径还原直接透传） |
| `api/handlers/admin/channels/probe.rs` | 新 `test_endpoint`；`probe_endpoint_raw` 增量读；删 `test_channel`/`detect_channel_quirks`/`analyze_response` 的 thinking 检测 |
| `api/handlers/admin/channels/types.rs` | 新 `TestEndpointRequest/Response`；删 `EndpointConfig.extras` + 旧 Test*/Detect*/recommendations |
| `relay/{cache,prepare,channel}.rs` | 清理 `extras: None` 占位 |
| `api/router.rs` | test-endpoint 路由；删 test/detect 路由 |
| `db/migrations/16_drop_channel_thinking_mode.sql` | DROP thinking_mode 列 |
| `frontend/.../ChannelForm.tsx` | 删 thinking 开关/检测区；端点卡片测试按钮 + 结果区 |
| `frontend/.../ChannelDetail.tsx` | 删测试按钮 |
| `frontend/.../api/types.ts` + `channels.ts` | 删 extras/recommendations；加 testEndpoint |

## 5. 验收契约（输入 → 期望输出）

| 输入 | 期望 |
|---|---|
| grep `thinking_normalizer\|PassthroughNormalizer\|ThinkingTagExtractor\|thinking_flag` src/ | 0 命中 |
| grep `extras` src/（除迁移文件） | 0 命中 |
| 迁移 16 执行后 channels 表 | 无 thinking_mode 列 |
| `cargo check` + `pnpm build` | 通过 |
| 点端点「测试」（健康端点） | success=true + 延迟/TTFT/tokens + thinking_detected，显示在该端点 Headers 下 |
| 点端点「测试」（上游限流） | success=false + 上游错误信息 |
| 测 openai_chat（GLM 类，主动开 thinking 才返回 `<think>`） | thinking_detected=true + 样本（payload 带 thinking 参数，不漏报） |
| 同渠道 3 端点并发测试 | 3 个独立结果，全部按上游真实状态判定（无误判） |
| ChannelForm | 无 thinking 开关 + 无思维链检测区 |
| ChannelDetail | 无「测试」按钮 |

## 6. 推进策略（分阶段）

| 阶段 | 内容 | 退出信号 |
|---|---|---|
| **P1 删思维链** | 删 thinking_normalizer.rs + stream_executor normalizer + analyze_response thinking 检测 + 迁移16 DROP thinking_mode + 删 EndpointConfig.extras（struct + 前端 type）+ 清理 extras 占位 | cargo check 通过 + grep 无 thinking_normalizer/extras 残留 |
| **P2 后端测试接口** | probe_endpoint_raw 增量读；新 test_endpoint + types + 路由；删 test_channel/detect + 旧 types | 单测：增量读 [DONE] 即停 + test_endpoint 返回连通性 |
| **P3 前端** | ChannelForm 删 thinking/检测区 + 端点卡片测试按钮/结果区；ChannelDetail 删测试按钮；api/types 更新 | pnpm build 通过 |
| **P4 验证** | cargo test 全量 + 手动并发测试 | 0 failed + 并发不再误判 |

P1（删思维链）独立可验证，先做；P2 测试接口；P3 前端；P4 验证。

## 7. 可卸载清单

拔掉这个 feature 要动：
1. 恢复 thinking_normalizer.rs + mod 声明 + stream_executor normalizer 挂载
2. 恢复 EndpointConfig.extras 字段 + 前端 type
3. 迁移 17 恢复 thinking_mode 列（或手动 ALTER ADD）
4. probe.rs：恢复 test_channel/detect_channel_quirks/analyze_response thinking 检测
5. 前端：恢复 thinking 开关 + 思维链检测区 + 渠道级测试按钮

DB 变更仅 1 个 DROP（迁移 16），卸载需对应 ADD（迁移不可回滚，只能追加 17）。
