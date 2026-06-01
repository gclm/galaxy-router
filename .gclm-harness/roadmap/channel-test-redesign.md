# 渠道测试重构 + Playground 优化方案

> 状态：待确认
> 日期：2026-06-01

## 背景

三个待解决的问题：

1. **API Key 反序列化 bug**：`test_model.rs` 把 `api_keys`（`Vec<UpstreamApiKey>`）反序列化为 `Vec<String>`，静默失败后报错"渠道没有配置 API Key"
2. **测试端点脱离渠道管理**：独立 `/test-model` 端点与渠道 CRUD 分离，不符合用户心智模型
3. **Playground 缺少调试能力**：无请求体预览、无参数调节、无配置持久化

## 参考分析

参考了 `new-api` 项目的渠道测试和 Playground 设计：

### 渠道测试（new-api）

| 维度 | new-api 做法 |
|------|-------------|
| 后端测试粒度 | `GET /api/channel/test/:id?model=xxx` 一次测一个模型 |
| 批量测试 | **前端驱动**：列出所有模型，用户勾选后 `Promise.allSettled` 并发调用后端 |
| Key 选择 | 不选，用渠道内置的随机/轮询选 key |
| 端点选择 | 前端下拉选 endpoint_type（auto/openai/anthropic/gemini 等） |
| 流式支持 | 前端开关，`stream=true` 时后端内部用流式请求测上游，仍返回单 JSON |
| 状态展示 | 每个模型独立状态（idle/testing/success/error）+ 响应时间 |

### Playground 对比

| 维度 | galaxy-proxy（当前） | new-api | 差距 |
|------|---------------------|---------|------|
| 多协议支持 | 5 种（chat/response/anthropic/embedding/image） | 固定 OpenAI chat | **galaxy-proxy 更强** |
| 流式 SSE | 支持，实时渲染 | 支持 | 持平 |
| 路由信息展示 | 有（渠道/模型/端点/耗时） | 无 | **galaxy-proxy 更强** |
| 多轮对话 | 不支持 | 支持 | 低优，不纳入 |
| 参数调节 | 无 | Temperature/TopP/MaxTokens 等 | **需补** |
| 请求体预览 | 无 | 有 | **需补** |
| 配置持久化 | 无 | localStorage 保存 | **需补** |

## 设计决策

| 决策点 | 选择 | 原因 |
|--------|------|------|
| 架构模式 | 后端薄 + 前端管批量 | 与 new-api 一致，后端简单可测，前端控制灵活 |
| Key 选择 | 用户选真实 `api_key` | 测试目的就是验证某条真实 key 能否请求端点；不再为上游 key 引入只服务测试的持久 ID |
| 端点选择 | 用户显式选 `test_protocol` | 渠道可能有多种端点，自动匹配不可靠 |
| 渠道测试流式 | 后端内部流式，返回单 JSON | 验证流式通路 + 记录首 token 耗时，前端无需处理 SSE |
| 旧端点 | 删除 `/test-model` | 合并到渠道管理，避免两套逻辑 |
| Playground 多轮 | 不做 | 定位是快速测试，非完整聊天 |

---

## 前置变更：保持 UpstreamApiKey 轻量

### 数据结构

```rust
pub struct UpstreamApiKey {
    pub key: String,
    pub note: String,
    pub enabled: bool,
}
```

不为渠道内的上游 API Key 单独生成持久 ID。真实代理请求按渠道内 enabled keys 轮询/重试；渠道测试为了定位具体 key 的连通性，直接传该真实 key，并由后端校验它属于当前渠道且启用。

### 前端展示规则

- **渠道列表**：不显示 key id（太长，无意义）
- **渠道详情/编辑**：显示 key 列表，每行显示 `note` + 脱敏 key（`...k8j6`）
- **渠道测试弹窗**：下拉选项显示 `note + 脱敏key`，选中后传真实 `api_key`
- **不可新增概念**：不引入 upstream key ID，避免迁移、隐藏字段回传、编辑保留 ID 等额外复杂度

### 数据迁移策略

`parse_api_keys` 函数只做旧格式兼容：

```rust
pub fn parse_api_keys(json_str: &str) -> Vec<UpstreamApiKey> {
    // 兼容 ["sk-xxx"] 和 [{"key":"sk-xxx","note":"","enabled":true}]
    // 不生成、不回写 upstream key id
}
```

运行库如已存在历史 `id` 字段，可通过一次性 SQL 清理 `channels.api_keys` JSON 中的 `id` 属性。

---

## 一、后端：渠道测试重构

### 端点

`POST /api/v1/admin/channels/{id}/test`

### 请求

```json
{
  "model": "mimo-v2.5",
  "test_protocol": "openai_chat",
  "api_key": "sk-...",
  "stream": true
}
```

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `model` | string | 是 | 要测试的模型名 |
| `test_protocol` | string | 是 | 端点协议类型，对应渠道 endpoints 的 type |
| `api_key` | string | 是 | 渠道 `api_keys` 中的真实 key；后端校验它属于该渠道且启用 |
| `stream` | boolean | 否 | 默认 false；true 时后端发流式请求测上游，记录首 token 耗时 |

### 响应

**成功（非流式）**：

```json
{
  "code": 0,
  "data": {
    "success": true,
    "message": "模型测试成功",
    "latency_ms": 1200,
    "input_prompt": "Hello! Please respond with a brief greeting in one sentence.",
    "output_content": "Hello! How can I help you today?"
  }
}
```

**成功（流式，stream=true）**：

```json
{
  "code": 0,
  "data": {
    "success": true,
    "message": "模型测试成功（流式）",
    "latency_ms": 1200,
    "time_to_first_token_ms": 180,
    "input_prompt": "Hello! Please respond with a brief greeting in one sentence.",
    "output_content": "Hello! How can I help you today?"
  }
}
```

**失败（上游错误）**：

```json
{
  "code": 0,
  "data": {
    "success": false,
    "message": "上游返回 HTTP 400: {\"error\":{\"message\":\"invalid model\"}}",
    "latency_ms": 500,
    "input_prompt": "Hello! Please respond with a brief greeting in one sentence.",
    "output_content": null
  }
}
```

**失败（参数错误）**：

```json
{
  "code": 400,
  "message": "指定的 API Key 不存在"
}
```

### 流式测试内部逻辑

当 `stream=true` 时：

1. 构建流式请求体（`"stream": true`）
2. 发送到上游，记录 `start_time`
3. 读取 SSE 流的第一个 `data:` 行，记录 `first_token_time = now - start_time`
4. 继续消费完整 SSE 流，拼接完整内容
5. 返回单 JSON 响应，包含 `time_to_first_token_ms` 和完整 `output_content`

### 后端实现步骤

**步骤 1：保持 `UpstreamApiKey` 无 id**

- `channels.rs` 中的 `UpstreamApiKey` 只保留 `key` / `note` / `enabled`
- `create` / `update` 原样序列化前端提交的 key 列表
- `parse_api_keys` 只兼容旧字符串数组，不再生成或回写 id

**步骤 2：在 `channels.rs` 新增 `test_channel` handler**

- 路径参数 `id` 取渠道 ID
- 请求体 `TestChannelRequest { model, test_protocol, api_key, stream }`
- 查渠道取 `api_keys`、`endpoints`、`custom_headers`
- 校验：渠道存在且启用、`api_key` 匹配到渠道内 enabled key、`test_protocol` 匹配到 endpoint
- 构建（流式/非流式）请求 → 发送到上游 → 解析响应

**步骤 3：复用 `test_model.rs` 的核心逻辑**

从现有 `test_model.rs` 搬迁到 `channels.rs`：

- `get_test_config(protocol, model)` → 构建请求体和路径
- `extract_content(resp_body, endpoint_type)` → 解析响应内容
- `inject_custom_headers(req_builder, headers)` → 注入自定义头
- `parse_protocol(protocol_str)` → 字符串转 `EndpointType`

新增：

- `send_streaming_test_request(...)` → 发流式请求，消费 SSE，返回首 token 时间 + 完整内容

**步骤 4：删除 `test_model.rs`**

- 删除 `src/api/handlers/admin/test_model.rs`
- 从 `src/api/handlers/admin/mod.rs` 移除 `pub mod test_model`
- 从 `src/api/router.rs` 移除 `TestModelState`、`test_model` import 和路由注册

**步骤 5：路由注册**

```rust
.nest(
    "/channels",
    Router::new()
        .route("/", get(channels::list).post(channels::create))
        .route(
            "/{id}",
            get(channels::get)
                .put(channels::update)
                .delete(channels::delete),
        )
        .route("/{id}/test", post(channels::test_channel))  // 新增
        .with_state(channel_state),
)
```

`TestModelState.http_client` 移入 `ChannelState`，或 `test_channel` 按需创建。

---

## 二、前端：渠道测试面板

参考 new-api 的 `ChannelTestDialog` 组件。

### 组件设计

**位置**：渠道编辑页内嵌测试入口（按钮触发弹窗或侧边面板）

**功能**：

1. **Key 选择**：下拉展示渠道所有 api_keys（显示 `note` + 脱敏 key 如 `...k8j6`），选中后传真实 `api_key`
2. **协议选择**：下拉展示渠道已配置的 endpoints（显示 `openai_chat` / `anthropic` 等）
3. **流式开关**：可选，开启后后端内部用流式请求测试
4. **模型列表**：展示渠道所有 models（支持搜索过滤）
5. **勾选 + 批量测试**：用户勾选模型 → 点击"测试 N 个模型" → 前端 `Promise.allSettled` 并发调用 `POST /channels/{id}/test`
6. **单模型测试**：每行一个"测试"按钮
7. **状态展示**：每个模型独立状态
   - 未测试（灰色）
   - 测试中（loading 动画）
   - 成功（绿色 + 响应时间 + 首 token 时间）
   - 失败（红色 + 错误信息）
8. **结果汇总**：全部完成后 toast 提示"N 成功 / M 失败"

### 数据流

```
渠道编辑页 → [测试] 按钮 → ChannelTestDialog
  ├─ 加载渠道数据（api_keys, endpoints, models）
  ├─ 用户选 key（传 api_key）+ protocol + stream
  ├─ 展示模型列表（可搜索/过滤）
  ├─ 用户勾选模型 → 批量测试
  │   └─ Promise.allSettled(
  │        models.map(m => POST /channels/{id}/test { model, test_protocol, api_key, stream })
  │      )
  └─ 每个模型独立更新状态
```

### API Key 展示规则

| 场景 | 展示内容 | 说明 |
|------|---------|------|
| 渠道列表 | 不显示 key | 信息密度高，无需 |
| 渠道详情 | `note` + `...k8j6`（脱敏） | 不显示完整 key |
| 渠道编辑 | `note` 输入框 + `key` 密码框 + `enabled` 开关 | 不携带 upstream key id |
| 测试弹窗 Key 下拉 | `note` + `...k8j6` | 选中后传真实 `api_key` |

---

## 三、前端：渠道详情页

当前渠道列表只有编辑和删除操作，缺少只读详情查看。新增渠道详情功能。

### 设计

**触发方式**：列表中点击渠道名称，或新增"详情"按钮

**展示方式**：弹窗（Dialog），只读，不可编辑

**展示内容**：

```
┌─────────────────────────────────────────┐
│ 渠道详情                          [关闭] │
├─────────────────────────────────────────┤
│ 基本信息                                 │
│ 名称:     MiMo 渠道                      │
│ 状态:     ● 启用                         │
│ 创建时间: 2026-05-29 10:00:00            │
│ 更新时间: 2026-06-01 14:30:00            │
├─────────────────────────────────────────┤
│ API Keys (2/3 启用)                      │
│ ┌─────────────────────────────────────┐ │
│ │ 默认 Key  ...k8j6  ● 启用           │ │
│ │ 备用 Key  ...m3p9  ● 启用           │ │
│ │ 已禁用    ...x7w2  ○ 禁用           │ │
│ └─────────────────────────────────────┘ │
├─────────────────────────────────────────┤
│ 端点 (2)                                 │
│ ┌─────────────────────────────────────┐ │
│ │ OpenAI Chat  https://xxx/v1  ● 启用 │ │
│ │ Anthropic    https://xxx/v1  ● 启用 │ │
│ └─────────────────────────────────────┘ │
├─────────────────────────────────────────┤
│ 模型 (8)                                 │
│ mimo-v2-omni  mimo-v2-pro  mimo-v2-tts  │
│ mimo-v2.5  mimo-v2.5-pro  ...           │
├─────────────────────────────────────────┤
│ 高级配置                                 │
│ 并发: 10  |  失败阈值: 3  |  黑名单: 10m │
│ RPM: -    |  TPM: -                      │
├─────────────────────────────────────────┤
│ 自定义请求头 (2)                          │
│ X-Custom-Header: value                   │
└─────────────────────────────────────────┘
```

### 实现要点

- **纯前端**：数据从列表已有的 channel 对象中取，不需要额外 API 调用
- **Key 脱敏**：只显示最后 4 位（`...k8j6`），不显示完整 key
- **底部操作栏**：提供"编辑"和"测试"快捷按钮，直接跳转到对应 Dialog
- **新组件**：`ChannelDetail` 组件，在 `Channels.tsx` 中引入

---

## 四、前端：Playground 优化

### 优化项

| 优化项 | 改动范围 | 说明 |
|--------|---------|------|
| 请求体预览 tab | `Playground.tsx` | 新增"请求体"tab，发送前预览将要发出的 JSON body |
| 基础参数 | `Playground.tsx` | 增加 `temperature`（0-2）和 `max_tokens`（1-4096）两个输入 |
| 配置持久化 | `Playground.tsx` | localStorage 保存选中状态（key/protocol/model/参数），刷新不丢失 |

### 请求体预览

当前有"渲染"和"原始"两个 tab，新增"请求体"tab：

```
[ 渲染 | 请求体 | 原始 ]
```

- 点击"请求体"tab 展示即将发送的 JSON（由 `buildRequestConfig` 生成）
- 实时更新：切换模型/协议/参数时自动刷新预览
- 不需要点击发送就能看到

### 参数调节

在配置面板的 Prompt 下方增加：

```
Temperature: [ 0.7 ]   (滑块或数字输入, 0-2, 步长 0.1, 可空)
Max Tokens:  [ 1024 ]  (数字输入, 1-4096, 可空)
```

- 只在支持的协议下显示（embedding/images 不显示）
- 值为空时不传入请求体（使用上游默认值）
- 拼入 `buildRequestConfig` 的 body 中

### 配置持久化

使用 `localStorage` key `playground-config` 保存：

```json
{
  "selectedApiKeyId": "uuid",
  "protocol": "openai_chat",
  "selectedModel": "gpt-4o",
  "temperature": 0.7,
  "maxTokens": 1024,
  "stream": true
}
```

- 页面加载时读取并恢复
- 每次变更时自动保存（带 debounce）

---

## 不做的事

- 不做多轮对话（Playground 定位是快速测试，非完整聊天）
- 不做消息编辑/重发/删除
- 不做渠道测试的 SSE 实时推送（后端内部流式 + 返回单 JSON 已足够）
- 不做自动定时测试（后续特性）
- 不做后端批量测试端点（前端驱动批量更灵活）

---

## 实施顺序

1. **前置：`UpstreamApiKey` 保持无 id** → 移除测试专用 key id + 清理旧 JSON id 字段
2. **后端渠道测试重构** → 删除旧端点，新增 `/channels/{id}/test`，支持流式
3. **前端渠道详情页** → 新 `ChannelDetail` 组件，只读弹窗
4. **前端渠道测试面板** → 新组件，集成到渠道编辑页
5. **前端 Playground 优化** → 请求体预览 + 参数 + 持久化
