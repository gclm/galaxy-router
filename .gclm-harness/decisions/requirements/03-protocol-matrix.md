# 协议支持矩阵

## 端点映射

| 协议 | 端点路径 | 请求格式 | 响应格式 | 协议转换 |
|------|---------|---------|---------|---------|
| OpenAI Chat Completions | `/v1/chat/completions` | JSON | JSON / SSE | ✅ 支持 |
| OpenAI Responses | `/v1/responses` | JSON | JSON / SSE | ✅ 支持 |
| Anthropic Messages | `/v1/messages` | JSON | JSON / SSE | ✅ 支持 |
| OpenAI Embedding | `/v1/embeddings` | JSON | JSON | ❌ 仅直通 |
| OpenAI Images | `/v1/images/generations` | JSON | JSON | ❌ 仅直通 |
| 模型列表 | `/v1/models` | - | JSON | - |

## 转换矩阵

| 入站 ↓ / 出站 → | OpenAI Chat | OpenAI Responses | Anthropic |
|-----------------|-------------|------------------|-----------|
| **OpenAI Chat** | ✅ 直通 | ⚠️ 需转换 | ⚠️ 需转换 |
| **OpenAI Responses** | ⚠️ 需转换 | ✅ 直通 | ⚠️ 需转换 |
| **Anthropic** | ⚠️ 需转换 | ⚠️ 需转换 | ✅ 直通 |

## 转换复杂度评估

### OpenAI Chat ↔ Anthropic（中等）

| 字段 | OpenAI Chat | Anthropic |
|------|-------------|-----------|
| 消息角色 | `system` / `user` / `assistant` | `user` / `assistant`（system 单独字段） |
| 内容类型 | `string` 或 `array` | `array`（content blocks） |
| 工具调用 | `tool_calls` 数组 | `tool_use` content block |
| 工具结果 | `role: "tool"` | `role: "user"` + `tool_result` block |
| 停止原因 | `finish_reason: "stop"` | `stop_reason: "end_turn"` |
| 流式事件 | `data: {...}` | `event: {...}\ndata: {...}` |

### OpenAI Chat ↔ OpenAI Responses（较复杂）

| 字段 | OpenAI Chat | OpenAI Responses |
|------|-------------|------------------|
| 消息结构 | `messages` 数组 | `input` 数组（支持多种 item 类型） |
| 输出格式 | `choices[].message` | `output[]` 数组（message / function_call / reasoning） |
| 流式事件 | `data: {choices: [...]}` | `event: response.xxx\ndata: {...}` |
| 工具定义 | `tools` 数组 | `tools` 数组（格式略有不同） |

### OpenAI Responses ↔ Anthropic（最复杂）

需要同时处理两种差异：
1. 消息结构差异（Responses 的 item 类型 vs Anthropic 的 content blocks）
2. 流式事件格式差异（Responses 的 event type vs Anthropic 的 event type）

## 统一内部模型（参考 AxonHub）

AxonHub 的 `llm/model.go` 定义了成熟的统一模型，以 OpenAI Chat Completion 为基准扩展：

```rust
// Galaxy Router 的 Rust 实现应镜像此设计
struct LlmRequest {
    messages: Vec<Message>,
    model: String,
    temperature: Option<f64>,
    max_tokens: Option<i64>,
    stream: Option<bool>,
    tools: Vec<Tool>,
    tool_choice: Option<ToolChoice>,
    reasoning_effort: Option<String>,
    // 辅助字段（不发送给 LLM）
    api_format: ApiFormat,
    transformer_metadata: HashMap<String, Value>,
}

struct Message {
    role: String,                        // user/assistant/system/tool/developer
    content: MessageContent,             // 字符串或多模态内容
    tool_calls: Option<Vec<ToolCall>>,
    reasoning_content: Option<String>,   // 推理内容
    reasoning_signature: Option<String>, // 加密推理内容
    cache_control: Option<CacheControl>,
    annotations: Option<Vec<Annotation>>,
}

struct LlmResponse {
    id: String,
    choices: Vec<Choice>,
    object: String,   // "chat.completion" 或 "chat.completion.chunk"
    usage: Option<Usage>,
    error: Option<ResponseError>,
    api_format: ApiFormat,
}
```

**设计要点**:
- 以 OpenAI 为基准减少转换量
- 辅助字段（`api_format`, `transformer_metadata`）隔离格式专属信息
- `MessageContent` 支持单字符串或 `[]ContentPart`（text/image_url/video_url）

## 双 Transformer 接口（参考 AxonHub）

```rust
// Inbound: 客户端格式 ↔ 统一内部格式
trait Inbound {
    fn transform_request(&self, req: &HttpRequest) -> Result<LlmRequest>;
    fn transform_response(&self, resp: &LlmResponse) -> Result<HttpResponse>;
    fn transform_stream(&self, stream: Stream<LlmResponse>) -> Result<Stream<StreamEvent>>;
    fn aggregate_stream_chunks(&self, chunks: &[StreamEvent]) -> Result<(Bytes, ResponseMeta)>;
}

// Outbound: 统一内部格式 ↔ 上游提供商格式
trait Outbound {
    fn api_format(&self) -> ApiFormat;
    fn transform_request(&self, req: &LlmRequest) -> Result<HttpRequest>;
    fn transform_response(&self, resp: &HttpResponse) -> Result<LlmResponse>;
    fn transform_stream(&self, stream: Stream<StreamEvent>) -> Result<Stream<LlmResponse>>;
    fn aggregate_stream_chunks(&self, chunks: &[StreamEvent]) -> Result<(Bytes, ResponseMeta)>;
}
```

## 转换路径（Pipeline）

```
客户端请求
  → Inbound.TransformRequest()     # 客户端格式 → 统一格式
  → Middleware 链 (配额/映射/选择)
  → Outbound.TransformRequest()    # 统一格式 → 上游格式
  → HTTP 执行 (流式/非流式)
  → Outbound.TransformResponse()   # 上游响应 → 统一格式
  → Inbound.TransformResponse()    # 统一格式 → 客户端格式
  → 返回客户端
```

## 流式处理要点

| 要点 | 说明 |
|------|------|
| SSE 格式 | OpenAI 用 `data: {...}`，Anthropic 用 `event: {...}\ndata: {...}` |
| 事件粒度 | OpenAI Chat 是 token 级，Responses 是 content part 级，Anthropic 是 content block 级 |
| 终止事件 | OpenAI Chat: `data: [DONE]`，Responses: `event: response.completed`，Anthropic: `event: message_stop` |
| Token 统计 | 需要从流式事件中解析 usage 信息 |
