# Sub2API 项目分析

## 项目定位

商业级 AI API 网关平台，用于分发和管理 AI 产品的订阅配额。

## 技术栈

- 后端: Go 1.26, Gin, Ent ORM, Google Wire
- 前端: Vue 3, Vite, TailwindCSS, Pinia
- 数据库: PostgreSQL
- 缓存: Redis

## 我们借鉴什么

### 1. 负载均衡策略（加权评分模型）

```go
score = Priority权重 * priorityFactor
      + Load权重 * loadFactor
      + Queue权重 * queueFactor
      + ErrorRate权重 * errorFactor
      + TTFT权重 * ttftFactor
```

**默认权重**: Priority: 1.0, Load: 1.0, Queue: 0.7, ErrorRate: 0.8, TTFT: 0.5

**选择策略**: Top-K 加权随机，防止单点垄断。

### 2. 统计维度设计

| 维度 | 字段 |
|------|------|
| 用户 | `user_id` |
| API Key | `api_key_id` |
| 模型 | `model`, `requested_model`, `upstream_model` |
| 时间 | `created_at` (timestamptz) |

**Token 细分**: input, output, cache_creation, cache_read

**成本细分**: input_cost, output_cost, cache_creation_cost, cache_read_cost, total_cost

### 3. 故障转移机制

```
上游请求失败
    │
    ├─ RetryableOnSameAccount == true 且重试次数 < 3
    │   → 同账号重试 (500ms 间隔)
    │
    ├─ 加入 FailedAccountIDs
    │
    ├─ SwitchCount >= MaxSwitches → FailoverExhausted
    │
    └─ 切换计数递增，换号重试
```

### 4. 并发控制

- 基于 Redis 有序集合实现分布式槽位管理
- 键格式: `concurrency:account:{accountID}`
- 最大并发数由 `account.Concurrency` 决定
- 等待队列: 粘性最大等待 3，兜底最大等待 100

### 5. 调度器快照缓存

- 初始全量重建
- Outbox 增量更新
- 定期全量重建防漂移
- DB 降级限流

## 我们不借鉴什么

| 功能 | 原因 |
|------|------|
| 用户系统 | 个人使用不需要 |
| 支付系统 | 不做商业化 |
| Web 管理面板 | 第一版用 TOML |
| PostgreSQL + Redis | 过重，SQLite 够用 |

## 关键代码路径

| 模块 | 路径 |
|------|------|
| 账号调度器 | `backend/internal/service/openai_account_scheduler.go` |
| 网关服务 | `backend/internal/service/gateway_service.go` |
| 并发控制 | `backend/internal/service/concurrency_service.go` |
| 故障处理 | `backend/internal/handler/failover_loop.go` |
| 统计类型 | `backend/internal/pkg/usagestats/usage_log_types.go` |
