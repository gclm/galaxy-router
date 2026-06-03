# 操练场（Playground）开发方案

## 目标

提供一个独立的测试界面，用真实客户端请求验证代理的**完整请求管线**：API Key 认证 → 分组路由 → 渠道选择 → 协议转换 → 上游请求，并展示请求结果和路由信息。

与渠道测试的区别：

| 维度 | 渠道测试 | 操练场 |
|------|----------|--------|
| 认证 | 渠道自身 Key | 系统 API Key |
| 路由 | 直连上游 | 经分组路由 |
| 协议 | 只测渠道拥有的端点 | 客户端协议可不同于上游协议 |
| 输出 | 连通性 + 模型响应 | 路由信息 + 模型响应 |
| 场景 | "这个渠道能不能通" | "客户端发 Anthropic 请求，代理能否正确路由到 OpenAI 渠道" |

## 核心设计：纯前端实现

**后端仅新增 `/v1/models` 认证检查**（已完成）。前端模拟真实客户端，直接调用 `/v1/chat/completions` 等代理端点。

### 工作流程

```
1. 前端获取数据：
   - API Key 列表 → apiKeysApi.list()（管理接口，含 api_key 明文）
   - 模型列表 → 模拟客户端请求 GET /v1/models（用选中的 API Key 认证，
     返回 OpenAI 标准格式的模型列表，数据来自代理的分组表）

2. 用户配置请求参数（协议、模型、API Key、prompt）

3. 前端构造请求体，直接 fetch 到代理端点：
   - OpenAI Chat       → POST /v1/chat/completions
   - OpenAI Responses  → POST /v1/responses
   - Anthropic         → POST /v1/messages
   - OpenAI Embedding  → POST /v1/embeddings
   - OpenAI Images     → POST /v1/images/generations
   使用选中的 API Key 按各协议官方认证方式发送（非管理 token）

4. 接收并展示响应（支持流式）

5. 请求完成后，调用 statsApi.logs() 取最近一条日志，
   获取 channel_name / group_id / endpoint_type 等路由信息
```

### 后端改动范围

仅 `src/api/handlers/proxy/models.rs` 新增 `ApiKeyAuth` 认证（已完成）。其余复用现有能力：

| 需求 | 现有能力 |
|------|----------|
| API Key 列表 | `GET /api/v1/admin/api-keys` 管理接口，返回 `api_key` 明文 |
| 模型列表 | `GET /v1/models` 代理端点，需 API Key 认证（已添加），返回 OpenAI 标准格式（id 即分组名） |
| 发送请求 | `/v1/chat/completions` 等端点已存在，vite 已代理 `/v1` |
| 路由追踪 | 请求日志含 `channel_id`、`channel_name`、`group_id`、`endpoint_type` |
| 请求/响应内容 | `statsApi.logDetail(id)` 含 `request_content`、`response_content` |

---

## 前端

### 1. 新增页面

文件：`frontend/src/pages/Playground.tsx`

#### 布局

```
┌─────────────────────────────────────────────────────┐
│  操练场                                              │
│  用真实客户端请求测试代理管线                           │
├──────────────────────┬──────────────────────────────┤
│  配置面板             │  结果面板                      │
│                      │                              │
│  客户端协议 [▼]       │  ┌─ 路由信息 ──────────────┐ │
│  模型名称 [____]      │  │ 分组: qwen3.6-plus      │ │
│  API Key   [▼]       │  │ 渠道: 阿里云 Coding      │ │
│  Prompt    [____]    │  │ 端点: OpenAI Chat        │ │
│  ☑ 流式输出           │  │ 耗时: 2350ms             │ │
│                      │  └────────────────────────┘ │
│  [▶ 发送请求]         │                              │
│  (请求中变为 [■ 停止]) │  [渲染] [原始]               │
│                      │  ┌────────────────────────┐ │
│                      │  │ 渲染视图:               │ │
│                      │  │ Hello! How can I help... │ │
│                      │  │ (流式输出逐字显示)        │ │
│                      │  ├────────────────────────┤ │
│                      │  │ 原始视图:               │ │
│                      │  │ data: {"choices":[{"…"}]}│ │
│                      │  │ data: {"choices":[{"…"}]}│ │
│                      │  │ data: [DONE]             │ │
│                      │  └────────────────────────┘ │
│                      │                              │
│                      │  ✓ 成功 · 200 · 1.2s         │
└──────────────────────┴──────────────────────────────┘
```

#### 请求构造逻辑

根据选中的协议，按各协议官方格式构造请求（不使用 SDK，用原生 fetch，保持请求完全可见）：

```typescript
const PROXY_PATHS: Record<string, string> = {
  openai_chat: '/v1/chat/completions',
  openai_response: '/v1/responses',
  anthropic: '/v1/messages',
  openai_embedding: '/v1/embeddings',
  openai_images: '/v1/images/generations',
}

// 按各协议官方规范构造认证头和请求体
function buildRequestConfig(protocol: string, apiKey: string, model: string, prompt: string, stream: boolean) {
  const defaultPrompt = prompt || 'Hello! Please introduce yourself briefly.'

  switch (protocol) {
    case 'openai_chat':
      return {
        path: PROXY_PATHS.openai_chat,
        headers: {
          'Content-Type': 'application/json',
          'Authorization': `Bearer ${apiKey}`,
        },
        body: {
          model,
          stream,
          messages: [{ role: 'user', content: defaultPrompt }],
        },
      }
    case 'openai_response':
      return {
        path: PROXY_PATHS.openai_response,
        headers: {
          'Content-Type': 'application/json',
          'Authorization': `Bearer ${apiKey}`,
        },
        body: {
          model,
          stream,
          input: defaultPrompt,
        },
      }
    case 'anthropic':
      return {
        path: PROXY_PATHS.anthropic,
        headers: {
          'Content-Type': 'application/json',
          'x-api-key': apiKey,
          'anthropic-version': '2023-06-01',
        },
        body: {
          model,
          stream,
          max_tokens: 1024,
          messages: [{ role: 'user', content: defaultPrompt }],
        },
      }
    case 'openai_embedding':
      return {
        path: PROXY_PATHS.openai_embedding,
        headers: {
          'Content-Type': 'application/json',
          'Authorization': `Bearer ${apiKey}`,
        },
        body: {
          model,
          input: defaultPrompt,
        },
      }
    case 'openai_images':
      return {
        path: PROXY_PATHS.openai_images,
        headers: {
          'Content-Type': 'application/json',
          'Authorization': `Bearer ${apiKey}`,
        },
        body: {
          model,
          prompt: defaultPrompt,
          n: 1,
          size: '1024x1024',
        },
      }
  }
}
```

发送请求：

```typescript
const config = buildRequestConfig(protocol, apiKey, model, prompt, stream)
const response = await fetch(config.path, {
  method: 'POST',
  headers: config.headers,
  body: JSON.stringify(config.body),
})
```

认证方式按官方协议：
- OpenAI 系列（Chat / Responses / Embedding / Images）：`Authorization: Bearer <api_key>`
- Anthropic：`x-api-key: <api_key>` + `anthropic-version: 2023-06-01`

代理中间件（`src/api/middleware/mod.rs`）已支持两种认证方式，无需后端改动。

#### 流式响应处理

三种协议的 SSE 格式不同，需要分别解析：

| 协议 | SSE 格式 |
|------|----------|
| OpenAI Chat | `data: {"choices":[{"delta":{"content":"..."}}]}` |
| OpenAI Responses | `event: response.output_text.delta` + `data: {"type":"output_text","delta":"..."}` |
| Anthropic | `event: content_block_delta` + `data: {"delta":{"text":"..."}}` |

**流式终止标记**：

| 协议 | 终止信号 |
|------|----------|
| OpenAI Chat | `data: [DONE]` |
| OpenAI Responses | `event: response.completed` |
| Anthropic | `event: message_stop` |

```typescript
// 按 SSE 规范逐行解析，根据协议类型提取 content delta
async function parseSSEResponse(
  reader: ReadableStreamDefaultReader<Uint8Array>,
  protocol: string,
  onChunk: (text: string) => void,
) {
  const decoder = new TextDecoder()
  let buffer = ''
  while (true) {
    const { done, value } = await reader.read()
    if (done) break
    buffer += decoder.decode(value, { stream: true })
    // 按 \n\n 分割 SSE events，按协议格式提取 delta content
  }
}
```

Embedding 和 Images 为非流式协议，直接解析 JSON 响应。

#### 非流式响应解析

各协议的非流式响应结构不同，需要分别提取展示内容：

| 协议 | 响应结构 | 提取路径 |
|------|----------|----------|
| OpenAI Chat | `{"choices":[{"message":{"content":"..."}}]}` | `choices[0].message.content` |
| OpenAI Responses | `{"output":[{"type":"message","content":[{"type":"output_text","text":"..."}]}]}` | `output[0].content[0].text` |
| Anthropic | `{"content":[{"type":"text","text":"..."}]}` | `content[0].text` |
| Embedding | `{"data":[{"embedding":[0.001, ...]}]}` | 维度数 + 前 10 个值的截断预览 |
| Images | `{"data":[{"url":"..."}]}` 或 `{"data":[{"b64_json":"..."}]}` | 直接渲染图片 |

#### 结果展示双视图

结果面板提供两个 Tab 切换：

- **渲染视图**（默认）：解析后的可读内容
  - Chat / Responses / Anthropic：Markdown 渲染的文本
  - Embedding：`维度: 1536 | 前 10 值: [0.001, 0.002, ...]`
  - Images：`<img>` 标签直接展示
- **原始视图**：完整的原始响应（流式时逐行显示 SSE，非流式时显示 JSON）

```tsx
<Tabs defaultValue="rendered">
  <TabsList>
    <TabsTrigger value="rendered">渲染</TabsTrigger>
    <TabsTrigger value="raw">原始</TabsTrigger>
  </TabsList>
  <TabsContent value="rendered">
    {/* 根据协议类型渲染：文本/图片/向量预览 */}
  </TabsContent>
  <TabsContent value="raw">
    <pre className="...">{rawResponse}</pre>
  </TabsContent>
</Tabs>
```

原始视图始终记录完整的 SSE 事件流或 JSON 响应体，方便调试协议细节。

#### 错误响应处理

两种协议的错误格式不同：

- OpenAI：`{"error":{"message":"...","type":"..."}}`
- Anthropic：`{"type":"error","error":{"type":"...","message":"..."}}`

展示时兼容两种格式，提取 `message` 字段显示。

#### 路由信息获取

请求完成后，查询最近日志获取路由信息：

```typescript
const logs = await statsApi.logs({ page: 1, page_size: 1, api_key_id: selectedApiKey.id })
const latestLog = logs.items[0]
// latestLog.channel_name, group_id, endpoint_type, latency_ms 等
```

#### 关键交互

- **客户端协议**：下拉选择，复用 `ENDPOINT_LABELS`（全部 5 种有代理路由的协议；Gemini 仅有上游渠道端点，无客户端代理路由，故不列出）
- **模型名称**：下拉选择，从 `GET /v1/models`（用选中的 API Key 认证）获取模型列表，同时支持手动输入。切换 API Key 时刷新模型列表。
- **API Key**：下拉选择系统内已启用的 API Key
- **流式输出**：默认勾选，非流式协议（Embedding/Images）自动禁用
- **Prompt**：可选，默认使用内置测试 prompt
- 结果面板展示路由信息 + 实时响应，终端风格渲染
- **请求控制**：请求进行中按钮变为"停止"，点击通过 `AbortController` 中断流式请求；进行中时禁用"发送请求"按钮防止并发

### 2. 文件变更清单

| 文件 | 变更 |
|------|------|
| `frontend/src/pages/Playground.tsx` | 新建，操练场页面 |
| `frontend/src/pages/index.ts` | 新增导出 |
| `frontend/src/components/layout/Sidebar.tsx` | 新增导航项 |

无需新增后端代码（`/v1/models` 认证已添加）。前端直接用原生 fetch 调用代理端点（`/v1/models`、`/v1/chat/completions` 等），完全模拟真实客户端。`apiKeysApi` 获取 key 列表，`statsApi` 获取请求后的路由日志。

### 3. 路由和导航

`frontend/src/pages/index.ts` 新增导出。

路由注册处新增：

```tsx
{ path: '/playground', element: <Playground /> }
```

`Sidebar.tsx` 导航栏新增（放在"请求日志"和"设置"之间）：

```tsx
{
  title: '操练场',
  href: '/playground',
  icon: FlaskConical,  // lucide-react
}
```

---

## 实施顺序

```
Phase 1: 核心功能
  ├── 1.1 Playground.tsx 页面（配置面板 + 请求构造 + 响应展示）
  ├── 1.2 模型下拉列表（通过 GET /v1/models 模拟客户端获取）
  ├── 1.3 流式 SSE 解析 + 实时渲染（三种协议分别处理）
  ├── 1.4 非流式响应解析（Chat/Responses/Anthropic/Embedding/Images）
  ├── 1.5 结果展示双视图（渲染视图 + 原始视图 Tab 切换）
  ├── 1.6 路由信息获取（请求完成后查日志）
  └── 1.7 Sidebar 导航 + 路由注册

Phase 2: 优化
  ├── 2.1 请求耗时精确计时（前端计时 vs 日志 latency_ms）
  └── 2.2 请求历史（localStorage 存储）
```

---

## 风险点

1. **CORS**：前端直接请求 `/v1` 端点。开发环境由 vite 代理解决；生产环境前后端同源部署，无 CORS 问题。
2. **API Key 安全**：API Key 明文已在管理接口中返回，操练场复用同一数据源，不新增暴露面。操作者本身是管理员。
3. **流式响应中断**：需处理网络中断和超时，前端设 120s 超时。
4. **日志匹配精度**：通过 `api_key_id` 筛选最近一条日志。并发场景下可能不精确，可加前端请求时间窗口辅助匹配。
