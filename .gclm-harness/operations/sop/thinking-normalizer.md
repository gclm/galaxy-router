# 思维链规范化器（Thinking Normalizer）实施方案

## 背景

galaxy-router 作为 AI API 网关，代理客户端与上游 LLM 供应商之间的请求。部分国产上游供应商的非标准行为导致思维链（thinking/reasoning）内容混入正文输出，影响客户端显示效果。

### 问题 1：`<think/>` 标签嵌入 content（MiniMax-M3）

MiniMax-M3 有时将思维链内容直接放在 `delta.content` 里，用 `<think/>` XML 标签包裹，而非放在独立的 `reasoning_content` 字段。

**生产日志证据**（`019e972f`，`openai_response` + `conversion` 路径）：
```json
{"content":"\n用户让我分析...让我先探索一下项目。\n</think\n>\n\n我来分析项目并查看 CI 报错情况。"}
```

标签可能**跨多个 SSE chunk**，需要状态机解析。影响直通和转换两条路径。

### 问题 2：Anthropic signature 位置不对（GLM-5.1）

GLM-5.1 的 Anthropic 端点将 `signature` 放在 `content_block_start` 的事件体里，而非发送独立的 `signature_delta` 事件。

**实测证据**（直接调 GLM 官方 API）：
```
content_block_start → {"type":"thinking","signature":"3d93fc8d26ce411e81f4c34d"}
thinking_delta(s)...
content_block_stop    ← 没有 signature_delta！
```

这导致多轮对话的 thinking 上下文链断裂。影响 Anthropic 协议的直通和转换路径。

---

## 方案概览

| 层级 | 文件 | 职责 |
|------|------|------|
| 核心模块 | `src/proxy/thinking_normalizer.rs`（新建） | `<think/>` 标签状态机 + Anthropic signature 修复 |
| 协议层 | `src/protocol/anthropic.rs` | 转换路径：解析 `content_block_start` 提取 signature + 转换器补发 `signature_delta` |
| 数据模型 | `src/protocol/model.rs` | `StreamChoice` 增加 `thinking_signature` 字段 |
| 集成层 | `src/proxy/execute.rs` | 两条路径接入 normalizer |
| 配置层 | DB 迁移 + `types.rs` + `channel.rs` + `mod.rs` | 渠道级 `thinking_mode` 配置 |

---

## 阶段 1：核心状态机

### 新建 `src/proxy/thinking_normalizer.rs`

#### 1.1 `<think/>` 标签状态机 `ThinkTagParser`

```rust
pub struct ThinkTagParser {
    state: TagParseState,
    pending: String,     // 跨 chunk 的部分标签缓冲
    thinking_buf: String, // 累积的 thinking 内容
    content_buf: String,  // 累积的正常内容
}

enum TagParseState {
    Normal,
    MaybeOpenTag,   // 读到 '<'，正在匹配 "think"
    InsideThink,    // 在 <think...> 标签内
    MaybeCloseTag,  // 读到 '<'，正在匹配 "/think"
}
```

**状态转换逻辑**（`feed(&mut self, chunk: &str)`）：
- `Normal` 遇到 `<` → 缓存到 `pending`，进入 `MaybeOpenTag`
- `MaybeOpenTag` 继续匹配 `think` → 匹配成功进入 `InsideThink`；失败 → 把 `pending` 追加到 `content_buf`，回到 `Normal`
- `InsideThink` 遇到 `<` → 缓存到 `pending`，进入 `MaybeCloseTag`；其他 → 追加到 `thinking_buf`
- `MaybeCloseTag` 匹配 `/think>` → 回到 `Normal`；失败 → 把 `pending` 追加到 `thinking_buf`，回到 `InsideThink`
- `flush(&mut self)` → 处理流结束时 `pending` 中残余的部分标签

**注意**：MiniMax 的标签格式是 `<think\n>`（带换行符）而非 `<think >`，匹配逻辑需兼容 `think\n*>` 和 `think>`。

#### 1.2 转换路径规范化器 `ThinkingTagExtractor`

```rust
pub struct ThinkingTagExtractor {
    parser: ThinkTagParser,
}

impl ThinkingTagExtractor {
    pub fn new() -> Self;
    /// 从 LlmStreamResponse 的 delta.content 中提取 <think/> 标签内容
    /// 返回 true 表示内容被修改
    pub fn extract(&mut self, response: &mut LlmStreamResponse) -> bool;
    /// 流结束时 flush 残余
    pub fn flush(&mut self, response: &mut LlmStreamResponse);
}
```

**`extract()` 逻辑**：
1. 取 `choice.delta.content`，若为 `Content::Text(text)` 则喂给 `parser`
2. 将 parser 输出的 `content_buf` 写回 `delta.content`
3. 将 parser 输出的 `thinking_buf` 追加到 `delta.reasoning_content`

#### 1.3 直通路径规范化器 `PassthroughNormalizer`

```rust
pub struct PassthroughNormalizer {
    tag_parser: ThinkTagParser,
    // Anthropic signature 修复
    pending_signature: Option<String>,
    signature_block_index: Option<u32>,
}

impl PassthroughNormalizer {
    pub fn new() -> Self;
    /// 处理单个 SSE 事件，返回可能修改后的事件列表
    pub fn process_sse(&mut self, raw_bytes: &[u8], endpoint: &EndpointType) -> Vec<Vec<u8>>;
    /// 流结束时 flush
    pub fn flush(&mut self) -> Vec<Vec<u8>>;
}
```

**OpenAI Chat 路径**（`process_sse`）：
1. 解析 SSE 行，提取 `data` JSON
2. 检查 `choices[0].delta.content`
3. 喂给 `tag_parser`，获取清理后的 content 和 thinking
4. 重建 JSON：修改 `content`，追加 `reasoning_content`
5. 重新序列化为 SSE 事件

**Anthropic 路径**（`process_sse`）：
1. 解析 SSE 行，提取 `event` 和 `data`
2. `content_block_start` 事件：检测 `content_block.signature`，缓存到 `pending_signature`，原样转发（去掉 signature 避免客户端重复处理）
3. `content_block_stop` 事件：若有 `pending_signature`，在 `content_block_stop` 之前注入一个 `signature_delta` 事件
4. 其他事件：原样转发

**性能保障**：当 `thinking_mode = Off` 时，不创建 normalizer，直通路径零开销。

---

## 阶段 2：协议层 Signature 修复

### 修改 `src/protocol/model.rs`

在 `StreamChoice` 结构体增加字段：

```rust
pub struct StreamChoice {
    pub index: u32,
    pub delta: Message,
    pub finish_reason: Option<FinishReason>,
    /// Anthropic thinking block signature（用于多轮 thinking 连续性）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_signature: Option<String>,
}
```

用 `#[serde(skip_serializing_if)]` 确保序列化时不影响其他协议。

### 修改 `src/protocol/anthropic.rs` — Outbound

**`transform_stream_event()`（当前 line ~506-640）**：

在 `match sse.event_type.as_str()` 中增加 `"content_block_start"` 分支（当前 line ~638 `_ => Ok(None)` 之前）：

```rust
"content_block_start" => {
    let cb_type = parsed["content_block"]["type"].as_str().unwrap_or("");
    if cb_type == "thinking" {
        let signature = parsed["content_block"]["signature"]
            .as_str().map(|s| s.to_string());
        Ok(Some(LlmStreamResponse {
            id: String::new(),
            // ... 标准字段 ...
            choices: vec![StreamChoice {
                index: parsed["index"].as_u64().unwrap_or(0) as u32,
                delta: Message { /* 全部 None */ },
                finish_reason: None,
                thinking_signature: signature,  // ← 新字段
            }],
            // ...
        }))
    } else {
        Ok(None)  // text block 的 start 事件忽略
    }
}
```

### 修改 `src/protocol/anthropic.rs` — AnthropicStreamConverter

**增加字段**：

```rust
pub struct AnthropicStreamConverter {
    // ... 现有字段 ...
    pending_thinking_signature: Option<String>,  // 新增
}
```

**在 `convert()` 方法中**（当前 line ~1013-1065）：

处理 `choice.thinking_signature`：存储到 `pending_thinking_signature`，不立即输出。

**修改 `close_thinking_block()`**（当前 line ~776-791）：

在发送 `content_block_stop` **之前**，补发 `signature_delta`：

```rust
fn close_thinking_block(&mut self) -> Vec<Vec<u8>> {
    let mut events = Vec::new();
    if self.has_thinking_content_started {
        // ← 新增：补发 signature_delta
        if let Some(sig) = self.pending_thinking_signature.take() {
            events.push(Self::make_sse_event("content_block_delta", serde_json::json!({
                "type": "content_block_delta",
                "index": self.content_index,
                "delta": { "type": "signature_delta", "signature": sig }
            })));
        }
        // 原有逻辑：发送 content_block_stop
        events.push(Self::make_sse_event("content_block_stop", ...));
        self.has_thinking_content_started = false;
        self.content_index += 1;
    }
    events
}
```

---

## 阶段 3：配置层

### 3.1 新建 `src/db/migrations/11_add_channel_thinking_mode.sql`

```sql
ALTER TABLE channels ADD COLUMN thinking_mode TEXT;
```

Nullable TEXT。`NULL` = 关闭（向后兼容），`'normalize'` = 启用规范化。

### 3.2 修改 `src/api/handlers/admin/channels/types.rs`

在以下四个结构体中增加 `thinking_mode: Option<String>`：

- `ChannelRow`（line 5-22）
- `Channel`（line 116-133）
- `CreateChannelRequest`（line 137-151）
- `UpdateChannelRequest`（line 155-169）

### 3.3 修改 `src/proxy/channel.rs`

在 `ChannelInfo`（line 7-18）增加：

```rust
pub thinking_mode: Option<String>,
```

### 3.4 修改 `src/proxy/mod.rs`

**`get_channel()`（line ~515）**：SQL 查询增加 `thinking_mode` 列。

当前元组类型是 `(String, String, String, String, String, String, i32, i32)`，改为 `(String, String, String, String, String, String, i32, i32, Option<String>)`。

**`find_channel_by_model()`（line ~573）**：同上。

**`ChannelInfo` 构造**：增加 `thinking_mode` 字段赋值。

---

## 阶段 4：集成到流式管道

### 修改 `src/proxy/execute.rs`

**在 `execute_proxy_stream()` 的 spawned task 中**：

#### 4.1 创建 normalizer（在 `let mut stream = std::pin::pin!(...)` 之后）

```rust
let thinking_normalizer: Option<Box<dyn ThinkingNormalizerTrait>> =
    match selection.channel.thinking_mode.as_deref() {
        Some("normalize") => Some(Box::new(PassthroughNormalizer::new())),
        _ => None,
    };
```

#### 4.2 转换路径集成（line ~829-831 之间）

```rust
// stats collection 之后，converter.convert() 之前
if let Some(ref mut extractor) = conversion_thinking_extractor {
    extractor.extract(&mut llm_stream);
}
```

`conversion_thinking_extractor` 是 `Option<ThinkingTagExtractor>`，在 needs_conversion 分支开头创建。

#### 4.3 直通路径集成（line ~930-934 之间）

将现有的：
```rust
if !stream_send(&stream_tx, bytes).await { break; }
```

改为：
```rust
if let Some(ref mut normalizer) = thinking_normalizer {
    let events = normalizer.process_sse(&event_bytes, &upstream_endpoint_clone);
    for evt in events {
        if !stream_send(&stream_tx, Bytes::from(evt)).await { break; }
    }
} else {
    if !stream_send(&stream_tx, bytes).await { break; }
}
```

**当 `thinking_mode = NULL` 时，走 else 分支，与现有行为完全一致，零开销。**

#### 4.4 流结束 flush

在两个路径的末尾（`drop(stream_tx)` 之前），调用 normalizer 的 `flush()` 处理残余。

---

## 阶段 5：测试

### 5.1 单元测试（在 `thinking_normalizer.rs` 底部）

| 测试 | 覆盖场景 |
|------|---------|
| `complete_tag_in_one_chunk` | 单 chunk 内完整 `<think/>` 标签 |
| `tag_split_across_chunks` | `<thi` + `nk>内容</thi` + `nk>` |
| `tag_split_at_angle_bracket` | 标签恰好在 `<` 处断裂 |
| `multiple_think_blocks` | 多个 thinking 块 |
| `no_tags_passthrough_unchanged` | 无标签时内容不变 |
| `malformed_tag_recovers` | `<think` 无闭合 `>` 时的恢复 |
| `flush_drains_pending` | 流结束时的残余处理 |
| `real_minimax_m3_pattern` | 生产日志中的实际格式 |
| `anthropic_signature_in_start` | GLM signature 提取 |
| `anthropic_no_signature_passthrough` | 正常 Anthropic 无 signature 时不受影响 |

### 5.2 集成测试（在 `execute.rs` tests 模块中）

复用已有的 `spawn_mock_upstream()` 模式：

| 测试 | 说明 |
|------|------|
| `minimax_with_think_tags_converted` | MiniMax 风格 `<think/>` → `reasoning_content` 正确分离 |
| `glm51_anthropic_signature_fixed` | GLM 风格 signature → `signature_delta` 补发 |
| `passthrough_without_normalization` | `thinking_mode = NULL` 时零改动直通 |

### 5.3 端到端验证

```bash
# 1. 构建 + 部署
make brew-deploy

# 2. 开启 minimax 渠道的 thinking_mode
sqlite3 /opt/homebrew/var/lib/galaxy-router/galaxy.db \
  "UPDATE channels SET thinking_mode = 'normalize' WHERE id = '019e812a-4a65-7991-9ce2-5701327894ed';"

# 3. 通过 curl 测试
# 4. 查日志确认
sqlite3 /opt/homebrew/var/lib/galaxy-router/galaxy.db \
  "SELECT response_content FROM usage_logs WHERE requested_model = 'MiniMax-M3' ORDER BY created_at DESC LIMIT 3;"
```

---

## 文件清单

| 文件 | 操作 | 行数估算 |
|------|------|---------|
| `src/proxy/thinking_normalizer.rs` | 新建 | ~250 行 |
| `src/proxy/execute.rs` | 修改（4 处插入点） | ~40 行改动 |
| `src/protocol/anthropic.rs` | 修改（outbound + converter） | ~50 行改动 |
| `src/protocol/model.rs` | 修改（加 1 个字段） | ~3 行改动 |
| `src/proxy/mod.rs` | 修改（SQL 查询） | ~10 行改动 |
| `src/proxy/channel.rs` | 修改（加 1 个字段） | ~3 行改动 |
| `src/api/handlers/admin/channels/types.rs` | 修改（4 个结构体） | ~8 行改动 |
| `src/db/migrations/11_add_channel_thinking_mode.sql` | 新建 | 1 行 |
| 测试代码 | 新建（单元 + 集成） | ~200 行 |
| **合计** | | **~565 行** |

## 实施顺序

```
阶段 1 → thinking_normalizer.rs 核心状态机 + 单元测试
阶段 2 → anthropic.rs 协议层 signature 修复
阶段 3 → 配置层（迁移 + struct + SQL）
阶段 4 → execute.rs 集成接入
阶段 5 → 集成测试 + brew-deploy 验证
```

每阶段完成后可独立测试和合并。
