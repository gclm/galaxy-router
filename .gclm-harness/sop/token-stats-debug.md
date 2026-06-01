# SOP: Token 统计缺失排查与修复

## 触发条件

某模型在管理面板或 usage_logs 中 `input_tokens` 和 `output_tokens` 均为 0，但请求 `status_code = 200` 且有实际响应。

## 诊断步骤

### 1. 确认 token 覆盖率

```bash
sqlite3 /opt/homebrew/var/lib/galaxy-router/galaxy.db \
  "SELECT requested_model, COUNT(*) as total,
    SUM(CASE WHEN input_tokens=0 AND output_tokens=0 THEN 1 ELSE 0 END) as zero_cnt
   FROM usage_logs
   WHERE date(created_at) >= date('now', '-1 day')
   GROUP BY requested_model ORDER BY total DESC;"
```

- `zero_cnt = 0` → 无问题
- `zero_cnt = total` → 全部缺失，继续排查
- `zero_cnt < total` → 部分缺失，可能是旧版本数据，检查日期分布

### 2. 检查请求是否真正成功

```bash
sqlite3 /opt/homebrew/var/lib/galaxy-router/galaxy.db \
  "SELECT status_code, error_message, substr(response_content, 1, 200)
   FROM usage_logs WHERE requested_model='<MODEL>' ORDER BY created_at DESC LIMIT 5;"
```

- `response_content` 含 `{"error": "..."}` → 上游返回错误，不是 token 提取 bug
- `status_code != 200` → 请求本身就失败了

### 3. 确认端点类型和协议

```bash
sqlite3 /opt/homebrew/var/lib/galaxy-router/galaxy.db \
  "SELECT endpoint_type, request_type, is_stream, COUNT(*)
   FROM usage_logs WHERE requested_model='<MODEL>'
   GROUP BY endpoint_type, request_type, is_stream;"
```

不同端点类型走不同的 token 提取路径。

### 4. 查看渠道配置（拿到上游 URL 和 Key）

```bash
# 从 attempts 中拿到 channel_id
sqlite3 /opt/homebrew/var/lib/galaxy-router/galaxy.db \
  "SELECT attempts FROM usage_logs WHERE requested_model='<MODEL>'
   ORDER BY created_at DESC LIMIT 1;"

# 用 channel_id 查渠道的端点配置
sqlite3 /opt/homebrew/var/lib/galaxy-router/galaxy.db \
  "SELECT id, name, endpoints FROM channels WHERE id='<CHANNEL_ID>';"

# 提取可用 key
sqlite3 /opt/homebrew/var/lib/galaxy-router/galaxy.db \
  "SELECT api_keys FROM channels WHERE id='<CHANNEL_ID>';" \
  | python3 -c "import sys,json; keys=json.loads(sys.stdin.read()); print([k['key'] for k in keys if k.get('enabled',True)][0])"
```

### 5. 跨模型对比（隔离是模型问题还是协议问题）

```bash
sqlite3 /opt/homebrew/var/lib/galaxy-router/galaxy.db \
  "SELECT requested_model, endpoint_type, COUNT(*) as total,
    SUM(CASE WHEN input_tokens=0 AND output_tokens=0 THEN 1 ELSE 0 END) as zero_cnt
   FROM usage_logs
   WHERE date(created_at) >= date('now', '-1 day')
   GROUP BY requested_model, endpoint_type ORDER BY endpoint_type, total DESC;"
```

- 同 `endpoint_type` 下其他模型有 token → 上游供应商对该模型不返回 usage 或 SSE 格式不同
- 同 `endpoint_type` 下所有模型都没 token → 协议级解析 bug

### 6. 直接 curl 上游验证 SSE 格式

```bash
# Anthropic 端点
curl -s -N '<URL>/v1/messages' \
  -H 'x-api-key: <KEY>' \
  -H 'anthropic-version: 2023-06-01' \
  -H 'content-type: application/json' \
  -d '{"model":"<MODEL>","max_tokens":1000,"stream":true,
       "messages":[{"role":"user","content":"hi"}]}' | head -30

# OpenAI 端点
curl -s -N '<URL>/v1/chat/completions' \
  -H 'Authorization: Bearer <KEY>' \
  -H 'content-type: application/json' \
  -d '{"model":"<MODEL>","max_tokens":1000,"stream":true,
       "stream_options":{"include_usage":true},
       "messages":[{"role":"user","content":"hi"}]}' | head -30
```

重点看：
- `event:` 和 `data:` 之间有无空格（`event:message_start` vs `event: message_start`）→ 影响 `sse_field()` 解析
- `message_start` 事件中 `usage` 的位置（`message.usage` 还是根级 `usage`）
- `message_delta` 事件中 `usage` 是否存在
- OpenAI 端点是否需要 `stream_options: {"include_usage": true}`

### 7. 定位提取失败的代码位置

关键函数在 `src/proxy/mod.rs`：

| 函数 | 作用 |
|------|------|
| `sse_field()` | SSE 行解析，兼容 `field:value` 和 `field: value` |
| `extract_usage_from_sse()` | 从 SSE 事件提取 usage（非流式用 `extract_usage`） |
| `collect_sse_content()` | 从 SSE 事件收集文本内容（用于 fallback 估算） |
| `estimate_tokens()` | 兜底估算，~3 字节/token |

### 8. 部署验证

```bash
make brew-deploy

# 发测试请求后查 DB
sqlite3 /opt/homebrew/var/lib/galaxy-router/galaxy.db \
  "SELECT requested_model, input_tokens, output_tokens, cache_read_tokens
   FROM usage_logs ORDER BY created_at DESC LIMIT 3;"
```

确认 `input_tokens > 0`、`output_tokens > 0`。

## 修复清单

**优先级：上游 usage > 兜底估算**

| 日期 | 问题 | 修复 |
|------|------|------|
| 2026-05-31 | DashScope SSE 无空格导致解析失败 | `sse_field()` 兼容 `field:value` 和 `field: value` |
| 2026-05-31 | 上游不返回 usage 时无兜底 | 添加 `estimate_tokens()` fallback |
| 2026-05-31 | 流式 cache_tokens 硬编码 0 | oneshot 通道传递 cache 值，spawn 使用实际值 |
