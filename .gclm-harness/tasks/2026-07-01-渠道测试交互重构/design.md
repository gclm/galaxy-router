---
doc_type: design
feature: 2026-07-01-渠道测试交互重构
status: approved  # 已实现，落地于 e93bf9f → 5304af6 → e43a2b1
complexity: complex
---

# 渠道配置简化：删除思维链处理 + 测试交互重构

> **最终实现（2026-07-02）**，经历两轮：
> - **Round 1（5304af6）**：删思维链运行时处理 + 端点卡片测试 + 修并发 bug
> - **Round 2（e43a2b1，实测后调整）**：接口去 `channel.id` + 测试回列表 TestModelDialog + content/reasoning 分离 + max_tokens 2048
> - **Round 3（本轮，未 commit）**：max_tokens 提至 4096（high effort 够）；ChannelForm 加测试入口（新建渠道不保存可测）
>
> **Round 2 起因**（实测发现）：① 端点卡片测试依赖 `channel.id`，**新增渠道（未保存）测不了**；② reasoning 模型（GLM/DeepSeek）thinking 走 `reasoning_content` 字段，probe 只提取 `content` → 「流式响应无内容」；③ `max_tokens:200` 太小，thinking 被截断、正文空。
>
> **关键区分**：运行时思维链处理（normalizer + extras 开关 + DB 列）**全删**；渠道测试保留思维链**诊断**（`reasoning` 字段 + `thinking_detected`），纯展示不驱动配置。

## 1. 背景与根因

### 1.1 思维链功能删除

galaxy 原有两套思维链处理（端点级 `extras.thinking.{extract_tags, fix_signature}`）：

| 开关 | 作用 | 针对场景 |
|---|---|---|
| extract_tags | 抽 `<think>...</think>` 标签到 reasoning_content | MiniMax-M3 等把 thinking 塞 content 的非标准上游 |
| fix_signature | 修 GLM-5.1 把 signature 放 content_block_start | GLM-5.1 anthropic signature 位置异常 |

**删除理由**：axonhub 调研证实标准 provider 走字段（reasoning_content/thinking block/signature_delta），不需标签解析；galaxy 当前上游无「塞 content」真实需求；线上 extras.thinking 为空。**全删**（含 extract_tags），等有真实非标准上游再重新设计。

**删除边界**：只删「运行时处理」（normalizer 应用 + extras 开关 + DB 列）。测试的「思维链检测」保留（诊断，与运行时处理无关）。

### 1.2 渠道测试交互问题

| # | 问题 | 根因 | 处理 |
|---|---|---|---|
| ① | 并发测试显示失败但实际成功 | `probe_endpoint_raw` 用 `resp.bytes().await` 等 SSE 连接关闭，keep-alive 超时误判 | **修**：bytes_stream 增量读，遇 `[DONE]` 即停 |
| ② | 思维链检测过度应用 | detect 渠道级合并 + 全端点应用 | **消失**：思维链功能删 |
| ③ | reasoning 模型「流式响应无内容」 | probe 只提取 `content`，GLM/DeepSeek thinking 走 `reasoning_content` | **修（Round 2）**：分别提取 content + reasoning |
| ④ | 渠道级测试语义混乱 | ChannelDetail「测试」实际只测单端点 | **修**：测试回列表 TestModelDialog（选协议/模型） |
| ⑤ | 新增渠道测不了 | 接口依赖 `channel.id` | **修（Round 2）**：接口去 id，请求体传完整参数 |

## 2. 目标

1. **删除思维链运行时处理**：thinking_normalizer.rs + stream_executor normalizer + DB thinking_mode 列 + EndpointConfig.extras + 前端 thinking 开关/检测区
2. **渠道测试 = 连通性 + 思维链诊断**：test-endpoint 返回 `output_content`（正文）+ `reasoning`（思维链）分两个字段，前端分开显示
3. **修并发 bug**：probe_endpoint_raw 改 bytes_stream 增量读
4. **测试入口在渠道列表**：TestModelDialog 对话框，接口不依赖 channel.id（新增/已存渠道都能测）
5. **max_tokens 2048**：reasoning 模型 thinking 不被截断，正文能生成

**明确不做**：不保留任何 thinking 运行时处理代码；测试诊断只展示不应用；不在端点卡片放测试（thinking 开关删了，端点卡片测试无逻辑依托）。

## 3. 方案（最终实现）

### 3.1 删除思维链运行时处理（已完成）

- 删 `src/protocol/thinking_normalizer.rs` + `mod.rs` 声明
- `stream_executor.rs`：删 normalizer import/thinking_flag/创建/使用（转换+直通路径还原直接透传）
- DB 迁移 16 `DROP COLUMN thinking_mode`
- 删 `EndpointConfig.extras`（后端 struct + 前端 type）+ 清理 `extras: None` 占位（cache/prepare/channel）
- 前端删 thinking 开关 + 思维链检测区

### 3.2 渠道测试接口（最终：去 id）

- `POST /api/v1/admin/channels/test-endpoint`（**不带 channel.id**，静态路由优先于 `/{id}`）
- 入参 `TestEndpointRequest`：`{ endpoint_type, base_url, api_key, model, headers?, user_agent? }`
- 后端 test_endpoint **不查渠道**，用请求体参数构造临时 `EndpointConfig` 探测 → 新增/已存渠道都能测
- 出参 `TestEndpointResponse`：`{ success, latency_ms, ttft_ms, output_content（正文）, reasoning（思维链）, error?, prompt_tokens, completion_tokens, thinking_detected, thinking_sample }`
- `probe_endpoint_raw` **分别累积**：
  - content（正文）：OpenAiChat=`delta.content`，OpenAiResponse=`delta`（output_text.delta），Anthropic=`delta.text`
  - reasoning（思维链）：OpenAiChat=`delta.reasoning_content`，Anthropic=`delta.thinking`，OpenAiResponse=未提取（局限）
  - **成功条件**：content 或 reasoning 任一非空（之前只看 content → reasoning 模型误判失败）
- 探测 payload（**各协议标准 reasoning 参数**）：OpenAiChat=`reasoning_effort:medium`，OpenAiResponse=`reasoning.effort:medium`，Anthropic=`thinking:{type:enabled,budget_tokens:1024}`；**max_tokens 2048**（OpenAiChat/Anthropic）/ max_output_tokens 2048（OpenAiResponse）
- `probe_endpoint_raw` 增量读（bytes_stream，遇 `[DONE]` 即停，不等连接关闭）
- 删 `test_channel` + `detect_channel_quirks` + 旧 types（TestChannel*/Detect*/EndpointDetection/recommendations）

### 3.3 前端（最终：测试回列表）

- **测试入口**：渠道列表每行测试按钮 + 详情页测试按钮 → TestModelDialog 对话框（选 API Key + 协议 + 搜索模型 → 单测/批量测）
- TestModelDialog（从 e93bf9f 恢复 + 改造）：调 `test-endpoint`（前端从渠道配置组装 base_url/headers），删无效的「流式测试」开关（test-endpoint 总流式）
- 终端分两段显示：`── 思维链 ──`（reasoning，黄）+ `── 输出 ──`（output_content，绿）
- ChannelForm 端点卡片**无测试按钮**（纯编辑表单）
- 删 thinking 开关 + 思维链检测区 + applyDetectedRecommendations

## 4. 改动清单（最终）

| 文件 | 改动 |
|---|---|
| `protocol/thinking_normalizer.rs` | **整文件删** |
| `protocol/mod.rs` | 删 mod 声明 |
| `relay/stream_executor.rs` | 删 normalizer 全链路（import/thinking_flag/创建/转换+直通使用） |
| `api/handlers/admin/channels/probe.rs` | test_endpoint（去 id，请求体构造 endpoint）；probe_endpoint_raw 增量读 + 分别累积 content/reasoning；build_probe_payload 标准 reasoning 参数 + max_tokens 2048；detect_thinking 检测 `<think>` |
| `api/handlers/admin/channels/types.rs` | TestEndpointRequest（+base_url/headers）/Response（+reasoning）；删 extras + 旧 Test*/Detect* |
| `relay/{cache,prepare,channel}.rs` | 清 extras 占位 |
| `api/router.rs` | `/test-endpoint`（去 id，静态优先）；删 test/detect 路由 |
| `db/migrations/16_drop_channel_thinking_mode.sql` | DROP thinking_mode |
| `frontend/api/types.ts` + `channels.ts` | TestEndpoint*（+base_url/headers/reasoning）；testEndpoint（去 id） |
| `frontend/components/TestModelDialog.tsx` | 恢复 + 改造（testEndpoint + content/reasoning 分段显示 + 删 useStream） |
| `frontend/components/ChannelDetail.tsx` | 恢复测试按钮 + onTest |
| `frontend/pages/Channels.tsx` | 恢复 testChannel state + TestModelDialog + 列表测试按钮 + onTest |
| `frontend/components/ChannelForm.tsx` | 删端点卡片测试（回退）+ thinking 开关/检测区 |

## 5. 验收契约

| 输入 | 期望 | 状态 |
|---|---|---|
| grep `thinking_normalizer\|PassthroughNormalizer\|thinking_flag` src/ | 0 命中 | ✅ |
| grep `extras` src/（除迁移） | 0 命中 | ✅ |
| 迁移 16 后 channels 表 | 无 thinking_mode | ✅ |
| cargo test 全量 | 0 failed（700+） | ✅ |
| `POST /channels/test-endpoint`（无 auth） | 401（路由存在） | ✅ |
| 测 openai_chat（GLM/DeepSeek reasoning 模型） | success=true + reasoning 有内容 + output_content 有正文 | ✅（max_tokens 2048 后） |
| 测 openai_chat（标准非 reasoning） | success=true + output_content 有内容 | ✅ |
| TestModelDialog 终端 | 思维链/输出分段显示 | ✅ |
| ChannelForm | 无 thinking 开关 + 无端点卡片测试 | ✅ |

## 6. 已知局限 / 遗留

| # | 局限/遗留 | 影响 | 处理 |
|---|---|---|---|
| 1 | `thinking_detected` 只检测塞 content 的 `<think>` 标签（MiniMax 类）；GLM/DeepSeek 走 reasoning 字段，其 thinking 由 `reasoning` 字段非空指示，`thinking_detected` 对它们是 false | 两套 thinking 指示并存（thinking_detected + reasoning 字段），略冗余 | 接受（用户认可） |
| 2 | ~~OpenAiResponse reasoning 未提取~~ 已补（`response.reasoning_summary_text.delta` → reasoning 字段） | — | ✅ 已补（f9a9d3c） |
| 3 | ~~max_tokens 2048 不够 high effort~~ 已提至 4096（OpenAiChat/OpenAiResponse/Anthropic 统一；Anthropic budget 2048） | — | ✅ 已调（Round 3） |
| 4 | ~~新增渠道测试需先保存~~ 已支持：ChannelForm 测试按钮用表单数据测（接口去 id，不保存） | — | ✅ 已支持（Round 3） |
| 5 | ~~vite.config.ts 未 commit~~ 已提交（proxy 29088→8080，dev 前端连 dev 后端） | — | ✅ 已提交 |
| 6 | `docs/design-client-config.md` + `docs/design-stats-refactor.md` 是**新功能设计草案**（client-config / stats-refactor），非本任务遗留；交接文档另写，新会话判断是否实现 | — | 📋 待新会话判断 |
| 7 | ~~plans/sessions 未 gitignore~~ 已加 `.gitignore` 规则 | — | ✅ 已加（f9a9d3c） |
| 8 | ~~P4-2 批量并发未压测~~ 已手动压测（用户验证通过，无误判） | — | ✅ 已验证 |

## 7. 可卸载清单

拔掉要动（按 commit 反向）：
1. e43a2b1 反向：test-endpoint 加回 id + 测试改回端点卡片 + content/reasoning 合并 + max_tokens 200
2. 5304af6 反向：恢复 thinking_normalizer.rs + stream_executor normalizer + extras 字段 + 迁移 17 恢复 thinking_mode + test_channel/detect_channel_quirks
3. 前端：恢复 thinking 开关 + 思维链检测区

DB 变更仅迁移 16（DROP thinking_mode），卸载需追加迁移 17（ADD）——迁移不可回滚只能追加。
