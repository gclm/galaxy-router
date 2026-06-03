# 核心需求

从三个参考项目提取，按优先级排列。

## P0 — 必须有

### 1. 协议互转

**来源**: 核心需求

| 入站协议（客户端 → Proxy） | 出站协议（Proxy → 上游） |
|---------------------------|------------------------|
| OpenAI Chat Completions | OpenAI Chat Completions |
| OpenAI Chat Completions | Anthropic Messages |
| OpenAI Responses | OpenAI Responses |
| OpenAI Responses | Anthropic Messages |
| Anthropic Messages | Anthropic Messages |
| Anthropic Messages | OpenAI Chat Completions |

**关键约束**:
- 同格式直通时（如 Anthropic→Anthropic），原始字节直接转发，不做解析
- 流式响应必须正确处理，SSE 事件逐 chunk 转发
- 保留原始请求的 `stream` 字段语义

### 1.1 额外端点支持

| 端点 | 协议 | 说明 |
|------|------|------|
| `/v1/embeddings` | OpenAI Embedding | 向量嵌入 API |
| `/v1/images/generations` | OpenAI Images | 图片生成 API |

**约束**:
- Embedding 和 Images 端点**仅支持 OpenAI 兼容的上游渠道**（`type = "openai_chat"` 或 `"openai_response"`）
- 如果请求的分组中没有 OpenAI 兼容渠道，返回 `400 Bad Request`，提示该模型不支持 Embedding/Images
- 不做协议转换（Anthropic 渠道不支持这些端点）

### 2. 分组配置（借鉴 Octopus）

**来源**: Octopus 的分组设计

```toml
[[groups]]
name = "claude-sonnet"           # 对外暴露的模型名
mode = "weighted"                # 负载均衡模式
match_regex = "^claude-sonnet.*" # 可选：正则匹配
retry_enabled = true
max_retries = 3

[[groups.items]]
channel_id = 1
model_name = "claude-sonnet-4-20250514"  # 上游实际模型名
priority = 1                              # 故障转移优先级
weight = 100                              # 加权分配权重
```

**负载均衡模式**:
- `round_robin` — 轮询
- `random` — 随机
- `failover` — 故障转移（按 priority）
- `weighted` — 加权分配

### 3. 负载均衡（借鉴 Sub2API）

**来源**: Sub2API 的加权评分模型

```
score = 1.0 * priority_factor
      + 1.0 * load_factor       # 1 - (load_rate/100)
      + 0.7 * queue_factor      # 1 - (waiting/max_waiting)
      + 0.8 * error_factor      # 1 - error_rate
      + 0.5 * ttft_factor       # 1 - normalize(ttft)
```

**选择策略**:
1. 取 Top-K（默认 7）最佳候选
2. 加权随机选择（防止单点垄断）
3. 粘性会话支持（通过 `session_hash` 路由到同一上游）

### 3.1 粘性会话

```toml
[sticky_session]
enabled = true
ttl_seconds = 3600        # 会话保持时间
```

**实现方式**:
- 请求携带 `session_hash` 参数
- 内存 HashMap 维护 `session_hash → channel_id` 映射（带 TTL）
- 会话过期后重新选择渠道
- 进程重启后粘性会话丢失（可接受，客户端会重建）

### 4. 统计功能（借鉴 Sub2API）

**来源**: Sub2API 的多维度统计

**统计维度**:
- API Key
- 模型（请求模型 / 实际模型）
- 时间（按天聚合）

**统计指标**:
- 请求次数（成功 / 失败）
- Token 用量（input / output / cache_read / cache_creation）
- 耗时（平均 / P99）

**存储**:
- SQLite `usage_logs` 表（只追加，不可变）
- 按天预聚合到 `usage_daily` 表

---

## P1 — 应该有

### 5. 渠道管理

```toml
[[channels]]
name = "anthropic-official"
type = "anthropic"                    # openai_chat | openai_response | anthropic
base_url = "https://api.anthropic.com"
api_keys = ["sk-ant-key1", "sk-ant-key2"]
failure_threshold = 3                 # 连续失败 N 次后拉黑
blacklist_minutes = 10                # 拉黑时长
```

### 6. 模型映射

```toml
[[channels]]
name = "openai-proxy"
type = "openai_response"
base_url = "https://proxy.example.com"

[[channels.model_map]]
source = "gpt-4o"           # 客户端请求的模型名
target = "gpt-4o-2024-08-06" # 实际转发的模型名

[[channels.model_map]]
source = "claude-*"          # 通配符匹配
target = "claude-3-5-sonnet"
```

### 7. 故障转移

- 连续失败 N 次后自动拉黑 M 分钟
- 拉黑期间自动跳过，定期自动恢复
- 同通道重试（可配置次数和间隔）

### 8. 并发 / 速率限制

```toml
[[channels]]
name = "rate-limited-channel"
rate_limit = { rpm = 60, tpm = 100000 }  # 每分钟请求数 / Token 数
concurrency = 10                           # 最大并发数
```

**实现方式**（单机部署，内存计数器）:
- `rpm`/`tpm`: 滑动窗口计数器（内存 HashMap + 定时清理）
- `concurrency`: Semaphore（tokio::sync::Semaphore）
- 进程重启后计数器重置（可接受）

---

## P2 — 可以有

### 9. 健康探测

定期向上游发送测试请求，自动标记不健康的渠道。

### 10. WebSocket 中继

支持 WebSocket 协议的上游服务（少数场景）。

### 11. API Key 管理

简单的 Key 生成/吊销，用于下游客户端认证。
