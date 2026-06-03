# AxonHub 项目分析

**核心价值**: Galaxy Router 的协议转换层应直接参考此项目的 `llm/` 模块。

## 项目定位

All-in-one AI Development Platform，Go 实现，`llm/` 是独立的 LLM 转换库。

## 技术栈

- 后端: Go 1.26, Gin, Ent ORM, GraphQL, uber/fx
- 前端: React + TypeScript + shadcn/ui
- 数据库: PostgreSQL / MySQL / SQLite

## 核心架构：双 Transformer 接口

```go
// Inbound: 客户端格式 ↔ 统一内部格式
type Inbound interface {
    TransformRequest(ctx, *httpclient.Request) (*llm.Request, error)
    TransformResponse(ctx, *llm.Response) (*httpclient.Response, error)
    TransformStream(ctx, Stream[*llm.Response]) (Stream[*httpclient.StreamEvent], error)
    TransformError(ctx, error) *httpclient.Error
    AggregateStreamChunks(ctx, []*httpclient.StreamEvent) ([]byte, ResponseMeta, error)
}

// Outbound: 统一内部格式 ↔ 上游提供商格式
type Outbound interface {
    APIFormat() llm.APIFormat
    TransformRequest(ctx, *llm.Request) (*httpclient.Request, error)
    TransformResponse(ctx, *httpclient.Response) (*llm.Response, error)
    TransformStream(ctx, Stream[*httpclient.StreamEvent]) (Stream[*llm.Response], error)
    TransformError(ctx, *httpclient.Error) *llm.ResponseError
    AggregateStreamChunks(ctx, []*httpclient.StreamEvent) ([]byte, ResponseMeta, error)
}
```

**借鉴点**: 接口设计清晰，Inbound/Outbound 职责分离，流式和非流式统一处理。

## 统一内部模型设计

以 **OpenAI Chat Completion** 为基础扩展：

```go
type Request struct {
    Messages            []Message
    Model               string
    Temperature         *float64
    MaxTokens           *int64
    Stream              *bool
    Tools               []Tool
    ToolChoice          *ToolChoice
    ReasoningEffort     string
    // 辅助字段（不发送给 LLM）
    APIFormat           APIFormat
    TransformerMetadata  map[string]any
    RawRequest          *httpclient.Request
}

type Message struct {
    Role                   string            // user/assistant/system/tool/developer
    Content                MessageContent    // 字符串或多模态内容
    ToolCalls              []ToolCall
    ReasoningContent       *string           // 推理内容
    ReasoningSignature     *string           // 加密推理内容
    CacheControl           *CacheControl
    Annotations            []Annotation
}

type Response struct {
    ID      string
    Choices []Choice
    Object  string   // "chat.completion" 或 "chat.completion.chunk"
    Usage   *Usage
    Error   *ResponseError
    APIFormat APIFormat
}
```

**借鉴点**: 以 OpenAI 为基准减少转换量，辅助字段隔离格式专属信息。

## 支持的 API 格式（15+）

| 格式 | 常量 |
|------|------|
| OpenAI Chat Completions | `openai/chat_completions` |
| OpenAI Responses | `openai/responses` |
| OpenAI Responses Compact | `openai/responses_compact` |
| OpenAI Image Generation | `openai/image_generation` |
| OpenAI Embedding | `openai/embeddings` |
| Anthropic Messages | `anthropic/messages` |
| Gemini Contents | `gemini/contents` |
| Vercel AI SDK | `aisdk/text`, `aisdk/datastream` |
| DeepSeek (Anthropic 格式) | — |
| 豆包 / 月之暗面 / Fireworks | — |
| Jina (Embedding/Rerank) | — |

## 转换路径（Pipeline）

```
客户端请求
  → Inbound.TransformRequest()     # 客户端格式 → 统一格式
  → Middleware 链 (配额/映射/选择/Prompt)
  → Outbound.TransformRequest()    # 统一格式 → 上游格式
  → HTTP 执行 (流式/非流式)
  → Outbound.TransformResponse()   # 上游响应 → 统一格式
  → Inbound.TransformResponse()    # 统一格式 → 客户端格式
  → 返回客户端
```

## 流式处理设计

```go
type Stream[T any] interface {
    Next() bool
    Current() T
    Err() error
    Close() error
}
```

**终端事件识别**:
- OpenAI Chat: `data: [DONE]`
- OpenAI Responses: `event: response.completed`
- Anthropic: `event: message_stop`

**持久化流**: 流式响应在 `Close()` 时聚合所有 chunk，写入数据库记录用量。

## 负载均衡策略

- **Adaptive**: 自适应（综合健康、延迟、权重、限流）
- **Failover**: 故障转移（权重 + 随机）
- **CircuitBreaker**: 熔断器（模型级别）

## 重试机制

- 同渠道重试: `CanRetry()` + `PrepareForRetry()`（最多 N 次）
- 跨渠道切换: `HasMoreChannels()` + `NextChannel()`（最多 N 次）
- 指数退避延迟

## 关键代码路径

| 模块 | 路径 |
|------|------|
| 统一模型 | `llm/model.go` |
| API 格式常量 | `llm/constants.go` |
| Transformer 接口 | `llm/transformer/interfaces.go` |
| OpenAI Inbound | `llm/transformer/openai/inbound.go` |
| OpenAI Outbound | `llm/transformer/openai/outbound.go` |
| OpenAI Responses | `llm/transformer/openai/responses/outbound.go` |
| Anthropic Inbound | `llm/transformer/anthropic/inbound.go` |
| Anthropic Outbound | `llm/transformer/anthropic/outbound.go` |
| 流接口 | `llm/streams/stream.go` |
| Pipeline | `llm/pipeline/pipeline.go` |
| 编排器 | `internal/server/orchestrator/orchestrator.go` |

## 对 Galaxy Router 的启示

1. **直接复用架构**: AxonHub 的 `llm/` 模块设计成熟，Galaxy Router 的 Rust 实现应镜像这套接口
2. **统一模型以 OpenAI 为基准**: 减少转换代码量
3. **流式处理用 trait**: Rust 中用 `trait Stream` 替代 Go 的接口
4. **Inbound/Outbound 分离**: 每个协议一对 transformer，职责清晰
5. **中间件模式**: 配额、映射、选择等逻辑通过中间件链插入
