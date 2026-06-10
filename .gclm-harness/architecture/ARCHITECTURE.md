# Architecture

## 模块总览

```
galaxy-router
├── main.rs              # 启动入口：加载配置 → 初始化 DB → 启动调度器 → 创建路由
├── config.rs            # AppConfig（TOML 加载 + 环境变量覆盖）
├── api/                 # HTTP 路由层
│   ├── router.rs        # 路由注册（proxy + admin）
│   ├── response.rs      # generate_id() 工具函数
│   ├── handlers/
│   │   ├── admin/       # 管理 API（CRUD: channels/groups/api-keys/stats/settings/backup）
│   │   └── proxy/       # 代理 API（chat/embeddings/images/messages/models/responses）
│   └── middleware/       # 认证（JWT + API Key）、CORS、Content-Type 校验
├── relay/               # ★ 代理请求生命周期
│   ├── state.rs         # ProxyState（共享状态：pool/http_client/lb/cache/rate_limiter/queue）
│   ├── pipeline.rs      # handle_proxy_request — 统一入口（限流→预算→权限→分发流/非流）
│   ├── candidates.rs    # build_relay_candidates — 构建候选列表（sticky + scored items）
│   ├── executor.rs      # ProxyRelayExecutor — 非流式执行（重试 + Key 轮换）
│   ├── stream_executor.rs # ProxyStreamRelayExecutor — 流式执行
│   ├── run.rs           # RelayRun / RelayStreamRun — 重试编排（capacity + attempt loop）
│   ├── converter.rs     # 协议转换调用（inbound→outbound）
│   ├── channel.rs       # ChannelInfo（上游渠道元数据）
│   ├── cache.rs         # ProxyCache（groups/channels/regex 三层缓存）
│   ├── queue.rs         # RequestQueue（排队 Semaphore）
│   ├── ratelimit.rs     # RateLimiter（RPM + TPM 令牌桶）
│   ├── prepare.rs       # 请求预处理
│   └── scheduler_task.rs # Scheduler（后台任务，启动调度器）
├── scheduler/           # ★ 负载均衡
│   ├── selector.rs      # select_channel — 加权随机选择 + Top-K
│   ├── scoring.rs       # 评分算法（priority/load/queue/error_rate/ttft）
│   ├── capacity.rs      # 渠道容量管理（并发槽位 + 熔断）
│   ├── circuit.rs       # 熔断器（failure_threshold → blacklist）
│   ├── state.rs         # LoadBalancerState（全局调度状态）
│   ├── runtime.rs       # RuntimeChannelState（单渠道运行时状态）
│   ├── sticky.rs        # 粘性会话（session_hash → channel_id 映射）
│   ├── metrics.rs       # 调度指标
│   └── trace.rs         # 调度追踪日志
├── protocol/            # ★ 协议转换
│   ├── inbound/         # 入站协议解析（openai_chat / openai_responses / anthropic）
│   ├── outbound/        # 出站协议构建（openai_chat / openai_responses / anthropic）
│   ├── model.rs         # 统一中间模型（UnifiedRequest / UnifiedResponse）
│   ├── sse.rs           # SSE 流解析与事件处理
│   ├── stream_converter.rs # 流式协议转换
│   └── thinking_normalizer.rs # 思维链规范化
├── metrics/             # 用量与指标
│   ├── model.rs         # ModelRegistry（模型元数据 + 定价）
│   ├── pricing.rs       # PricingRefresher（远程定价刷新）
│   ├── usage/           # Token 用量估算
│   ├── recorder/        # StatsRecorder（请求日志写入 + 脱敏）
│   ├── query/           # StatsState（统计查询）
│   └── attempt.rs       # 尝试统计
├── auth/                # 认证模块
│   ├── jwt.rs           # JWT 签发/验证
│   └── password.rs      # Argon2 密码哈希
├── error/                # 统一错误类型
│   ├── app.rs           # ApiError + ApiResponse（管理 API 统一响应）
│   └── proxy.rs         # ProxyError + ErrorClass + ErrorFormat + 格式化
├── db/                  # 数据库
│   ├── mod.rs           # Database（连接池 + 迁移）
│   └── migrations/      # SQL 迁移文件（1-12）
└── static_assets.rs     # 嵌入前端静态文件（rust-embed）
```

## 核心数据流

```
客户端请求
    │
    ▼
api/handlers/proxy/{chat,messages,responses,...}
    │  提取 API Key 认证 → ApiKeyAuth
    ▼
relay/pipeline.rs::handle_proxy_request()
    │  ① RPM/TPM 限流检查
    │  ② 预算检查（月/日额度）
    │  ③ 模型权限验证（三段式：403→404→503）
    │  ④ 分发流/非流
    ▼
relay/pipeline.rs::proxy_request() / proxy_stream()
    │
    ▼
relay/candidates.rs::build_relay_candidates()
    │  按 model 查 group → group_items → 转为 candidates
    │  sticky 优先，其余 scored 排序
    ▼
relay/run.rs::RelayRun.execute()
    │  遍历 candidates，逐个尝试：
    │    ① capacity.acquire() 拿并发槽
    │    ② executor.relay_to_candidate() 执行
    │    ③ 失败时按 ErrorClass 决定：换 Key / 换渠道 / 终止
    ▼
relay/executor.rs::relay_to_candidate()
    │  ① 查渠道信息（带缓存）
    │  ② 选择 endpoint（按入站类型匹配）
    │  ③ API Key 轮换
    │  ④ protocol/inbound 解析 → protocol/model 统一模型 → protocol/outbound 构建
    │  ⑤ 发送到上游
    ▼
上游响应
    │  ⑥ 记录 attempt stats + 写 usage_logs
    ▼
返回客户端
```

## 依赖关系（模块间）

```
api ──→ relay ──→ scheduler
  │         │         │
  │         ├──→ protocol (inbound/outbound)
  │         ├──→ metrics (recorder)
  │         └──→ db
  │
  ├──→ auth (JWT/password)
  ├──→ config
  └──→ static_assets (前端)
```

## 关键不变量

1. **缓存一致性**：渠道/分组 CRUD 后必须 `cache.invalidate()`，否则代理请求用的是旧数据
2. **Key 轮换不重复**：`api_key_attempts()` 用 `AtomicU64` 计数器确保同渠道多 Key 时均匀分布
3. **熔断阈值**：渠道连续失败 N 次（`failure_threshold`）进入黑名单，`blacklist_minutes` 后自动恢复
4. **粘性会话**：同一 `x-session-hash` 在 TTL 内固定同一渠道，避免对话上下文中断
5. **API Key 认证双路径**：支持 `Authorization: Bearer` 和 `x-api-key` header

## 已知技术债

（无）
