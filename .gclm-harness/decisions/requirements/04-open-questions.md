# 待确认问题 — 已确认

## 决策记录

| 问题 | 决策 | 备注 |
|------|------|------|
| Q1: 配置格式 | **TOML** | 只保留一种，简洁严格 |
| Q2: 配置热更新 | **Web 面板实时生效** | 渠道/分组等业务配置通过 Web 面板管理，实时生效；基础配置（TOML）重启生效 |
| Q3: OpenAI Chat Completions | **必须支持** | 核心渠道来源协议 |
| Q4: Embedding API | **支持** | `/v1/embeddings` |
| Q5: Images API | **支持** | `/v1/images/generations` |
| Q6: 粘性会话 | **需要** | `session_hash` 路由到同一上游 |
| Q7: 排队机制 | **建议：可配置** | 默认返回 429，可选开启排队（见下文建议） |
| Q8: 统计粒度 | **两者都保留** | `usage_logs` 原始日志 + `usage_daily` 按天聚合 |
| Q9: 成本计算 | **需要** | 使用 https://models.dev/api.json 的定价数据 |
| Q10: 部署方式 | **两者都支持** | 单二进制 + Docker |

---

## Q7 建议：排队机制

**建议采用可配置方案**：

```toml
[server.queuing]
enabled = false          # 默认关闭，直接返回 429
max_queue_size = 100     # 最大排队请求数
queue_timeout_secs = 30  # 排队超时时间
```

**理由**：
- 个人使用场景：直接 429 更简单，客户端可自行重试
- 团队使用场景：排队可平滑突发流量，避免大量重试冲击
- 可配置让用户按需选择

**实现优先级**：P2（先实现 429，后期加排队）

---

## 最终协议支持矩阵

| 入站 ↓ / 出站 → | OpenAI Chat | OpenAI Responses | Anthropic |
|-----------------|-------------|------------------|-----------|
| **OpenAI Chat** | ✅ 直通 | ⚠️ 转换 | ⚠️ 转换 |
| **OpenAI Responses** | ⚠️ 转换 | ✅ 直通 | ⚠️ 转换 |
| **Anthropic** | ⚠️ 转换 | ⚠️ 转换 | ✅ 直通 |

**额外端点**：
- `/v1/embeddings` — Embedding API
- `/v1/images/generations` — Images API

---

## 成本计算方案

**数据源**: https://models.dev/api.json

**实现方式**：
1. 启动时拉取 models.dev 数据，缓存到内存
2. 定期刷新（可配置间隔，默认 24h）
3. 本地覆盖：支持 TOML 配置自定义定价

```toml
[pricing]
source = "models.dev"           # 数据源
refresh_interval_hours = 24     # 刷新间隔
local_override = true           # 允许本地覆盖

[[pricing.models]]
model = "claude-sonnet-4-20250514"
input_per_million = 3.0
output_per_million = 15.0
cache_read_per_million = 0.3
```
