# Galaxy Router v1.1 架构重构与功能规划（分批次发布）

> 日期: 2026-07-13
> 状态: 设计阶段

## 1. 背景与目标

### 1.1 问题诊断

当前项目（v1.0.3）存在三个核心问题：

**架构层面：Handler 直接写 SQL，缺乏分层**

```
当前调用链:
  HTTP Request → axum Handler → sqlx::query(SQL) → SQLite
                                    ↑
                         业务逻辑、SQL构造、错误转换全堆在一起
```

Handler 文件膨胀（groups.rs 612 行、stats.rs 314 行），职责混杂（stats.rs 同时管统计查询和预算管理），无法独立测试业务逻辑。

**大文件问题（TOP 12）**

| 文件 | 行数 | 问题 |
|---|---:|---|
| `metrics/query/mod.rs` | 1202 | 全部统计查询塞在一个文件：概览/模型/渠道/日/小时/日志分页/详情/Key聚合/百分位 + 测试 |
| `relay/stream_executor.rs` | 1072 | 流式执行器：SSE解析+错误处理+usage注入+重试逻辑 |
| `protocol/inbound/openai_responses.rs` | 932 | Responses 协议入站解析 |
| `protocol/inbound/anthropic.rs` | 747 | Anthropic 协议入站解析 |
| `relay/executor.rs` | 720 | 非流式执行器 |
| `relay/pipeline.rs` | 696 | 代理管道 |
| `metrics/model.rs` | 694 | 模型定价注册表 |
| `metrics/recorder/mod.rs` | 639 | 统计记录器：结构体+DB写入+流式完成记录+测试 |
| `protocol/sse.rs` | 634 | SSE 解析 |
| `api/handlers/admin/groups.rs` | 612 | 分组CRUD |
| `relay/prepare.rs` | 549 | 请求准备 |
| `scheduler/state.rs` | 539 | 调度器状态 |

**功能缺失**

- 无请求拦截改写能力（cch清理/隐私清洗/思维链修复等）
- 统计可视化薄弱（有数据但缺深度展示）
- 无 Key 持有者自助查看页面
- "分组"命名不准确，应改为"模型路由"

### 1.2 设计目标

1. 引入分层架构，Handler 变薄，SQL 下沉到 Repository
2. 拆分所有 >500 行的大文件
3. 新增插件系统（请求拦截改写）
4. 统计模块增强（响应内容记录 + 可视化）
5. Key 持有者门户（`/portal` 轻量页面，key 换短期 token 鉴权）
6. 分组 → 模型路由全链路重命名

### 1.3 兼容性

不考虑管理 API 向后兼容（代理 API `/v1/*` 保持不变）。DB 迁移不可逆。

---

## 2. 架构设计：分层 + Repository + 模块化领域边界

### 2.1 架构选型

| 架构 | 评估 | 结论 |
|---|---|---|
| DDD（聚合/值对象） | 过重。项目核心是数据管道（请求转发），不是复杂领域逻辑 | 不选 |
| 六边形（Ports & Adapters） | Rust trait 适合做 port，但全套适配器对中等项目过度 | 不选 |
| 洋葱架构 | 分层严格但层太多 | 不选 |
| **分层 + Repository** | 最务实。Controller → Service → Repository，清晰且不过度 | **选定** |

核心原则：**模块化单体 + Repository 模式**。Rust 的模块系统天然提供边界，不需要 Java 那种重架构。

### 2.2 目标目录结构

```
src/
├── domain/               # 领域模型（纯 struct/trait，零框架依赖）
│   ├── mod.rs
│   ├── channel.rs        #   Channel, Endpoint, UpstreamKey
│   ├── route.rs          #   Route, RouteItem（原 group/group_item）
│   ├── api_key.rs        #   ApiKey, BudgetLimit
│   ├── usage.rs          #   UsageLog, UsageStats, AttemptStats
│   └── proxy.rs          #   ProxyRequest（纯入站请求数据；ProxyResult 含错误，移 llm/relay/）
│
├── repository/           # 数据访问层（SQL 隔离在此）
│   ├── mod.rs            #   Repositories（统一持有所有 repository）
│   ├── channel_repository.rs  #   ChannelRepository trait + impl
│   ├── route_repository.rs    #   RouteRepository trait + impl
│   ├── api_key_repository.rs  #   ApiKeyRepository trait + impl
│   ├── usage_repository/ #   UsageRepository trait + impl（目录，拆 1202 行 query/mod.rs，见 §3.4）
│   ├── budget_repository.rs   #   BudgetRepository trait + impl
│   └── settings_repository.rs #   SettingsRepository trait + impl
│
├── service/              # 业务逻辑层（无 HTTP 耦合）
│   ├── mod.rs
│   ├── pricing/          #   模型定价（metrics/model.rs + pricing.rs 合并）
│   └── stats/            #   统计聚合 + 记录器 + 脱敏
│       ├── mod.rs        #     StatsService（调用 usage_repository）
│       ├── recorder.rs   #     从 metrics/recorder 迁移
│       ├── redaction.rs  #     脱敏（从 metrics/recorder/redaction.rs 迁移）
│       ├── overview.rs
│       └── trend.rs
│
├── llm/                  # ★ LLM 转发引擎聚合（对齐 axonhub llm/，见 §2.3）
│   ├── relay/            #   转发管道（编排者，调下面三个）
│   │   ├── pipeline.rs   #     请求管道
│   │   ├── executor.rs   #     非流式执行（<300 行）
│   │   ├── stream_executor.rs #  流式执行（<400 行）+ thinking 流式钩子（§3.3.5）
│   │   ├── prepare.rs    #     请求准备 + 请求侧插件钩子（§3.3.4）
│   │   └── stream_accumulator.rs #  SSE 累积（§4.2）
│   ├── protocol/         #   协议转换（inbound 解析 / outbound 构造 / sse / converter）
│   │   ├── inbound/      #     入站解析（每个协议 <500 行）
│   │   ├── outbound/
│   │   ├── sse.rs
│   │   └── converter.rs #   协议间转换桥（如 openai_chat ↔ anthropic）
│   ├── scheduler/        #   负载均衡（选渠道）
│   └── plugin/           #   请求/响应拦截改写（规则；钩子在 relay 调用）
│       ├── mod.rs        #     RequestPlugin / ResponsePlugin trait + PluginChain
│       ├── cch.rs        #     Claude Code cch 标记清理（缓存命中 0%→97%）
│       ├── tracking.rs   #     隐私跟踪标记清洗
│       ├── cache_key.rs  #     prompt_cache_key 粘性路由注入
│       └── thinking.rs   #     思维链处理（字段规范化 + 内容分离）
│
├── api/                  # HTTP 接入层（axum）
│   ├── mod.rs
│   ├── router.rs         #   路由注册
│   ├── response.rs       #   统一响应格式
│   ├── middleware/       #   中间件
│   ├── admin/            #   管理 API（handler 变薄，只做参数校验+调service）
│   │   ├── mod.rs
│   │   ├── channels.rs   #     渠道管理 CRUD
│   │   ├── routes.rs     #     模型路由管理（原 groups）
│   │   ├── api_keys.rs   #     API Key 管理
│   │   ├── stats.rs      #     统计查询（纯查询，不含预算）
│   │   ├── budget.rs     #     预算管理（从 stats.rs 拆出）
│   │   ├── settings.rs
│   │   └── system.rs
│   ├── proxy/            #   代理 API（/v1/* 保持原生协议格式）
│   │   ├── mod.rs
│   │   ├── chat.rs       #     /v1/chat/completions
│   │   ├── messages.rs   #     /v1/messages
│   │   ├── responses.rs  #     /v1/responses
│   │   ├── models.rs     #     /v1/models
│   │   ├── images.rs     #     /v1/images/generations
│   │   └── embeddings.rs #     /v1/embeddings
│   └── portal/           #   ★ 新：Key 持有者门户
│       ├── mod.rs
│       └── handlers.rs   #     /portal 用量查看（key 换 token 鉴权，§4.4.1）
│
├── infra/                # 基础设施
│   ├── mod.rs
│   ├── db/               #   连接池、迁移执行器（SQL 文件留在 src/db/migrations，对齐约束 #5）
│   │   └── mod.rs
│   ├── cache.rs          #   ProxyCache（原 relay/cache.rs）
│   ├── config.rs         #   配置加载
│   └── presets.json      #   供应商预设模板（§4.5.3）
│
└── main.rs               # 入口（组装各层依赖）
```

### 2.3 依赖方向

```
            ┌─→ llm（relay 编排 → protocol / scheduler / plugin）─→ infra(reqwest)
api  ───────┤
            └─→ service ──→ repository ──→ domain
```

- **domain**：纯数据结构 + 业务规则。允许 `serde`（数据形状描述，非框架耦合），禁止 `axum/sqlx/reqwest` 等运行时框架。
- **repository**：trait 定义数据访问接口，SQLite 实现隔离在此。
- **service**：业务逻辑编排（pricing / stats），调用 repository trait，不碰 HTTP。
- **llm**：LLM 转发引擎聚合层，与 service 平级。内部 `relay`（编排）→ `protocol`（转换）/ `scheduler`（选渠道）/ `plugin`（改写）。依赖 infra（reqwest）——转发引擎碰 HTTP 是本职，不是"service 碰 HTTP"。
- **api**：HTTP handler，只做参数解析 + 调 service/llm + 响应格式化。
- **infra**：DB 连接池、缓存、配置等基础设施。

> **relay / protocol / scheduler / plugin 聚合到 `llm/`**，不散在顶层、也不塞进 service。调研四个参考 LLM 网关，无一把转发链路做成多个平级顶层包，也无一塞进"不碰 HTTP 的 service"：
>
> | 项目 | 转发 / 协议转换位置 | 组织 |
> |---|---|---|
> | **axonhub** (Go) | `llm/{pipeline, transformer, streams, ...}` | **`llm/` 大聚合**（galaxy 采纳此模式） |
> | octopus (Go) | `internal/relay/{balancer, transformers}` | relay 内含转换 + 调度 |
> | new-api (Go) | `relay/{*_handler}` | protocol 进 relay |
> | gt_ai_gateway (TS) | `src/service/` 直接转发 | service 碰 HTTP |
>
> galaxy 耦合度实测：`relay → protocol` 11 处、`relay → scheduler` 11 处，protocol/scheduler 互不依赖且外部消费者只有 relay——三者都是 relay 的子系统，聚合进 `llm/` 后 relay 调它们从跨顶层变成引擎内部调用，依赖最内聚；`plugin` 同理（钩子在 relay），一并纳入。

#### 2.3.1 错误类型分层

按"返回格式"而非"代码层级"划分，只保留两类错误（对齐约束 #2 #3）：

| 错误类型 | 含义 | 返回方式 | 适用 API |
|---|---|---|---|
| `ProxyError`（沿用 `error::proxy`） | 代理转发链路错误：上游非 2xx、协议转换失败、网络超时、relay 内部错误 | **原生格式透传**，不统一包装 | 代理 API `/v1/*` |
| `ServiceError`（`error::service`，合并现 `error::app`） | 服务侧错误：DB 故障、参数校验、业务规则、配置错误 | **统一封装** `{code, message, data}` | 管理 API `/api/v1/admin/*`、portal API |

- repository 层**不单独定义错误**：sqlx 错误在 service 层直接映射为 `ServiceError`，少一层转换。
- 现有 `error::app::ApiError`（错误）→ 重命名为 `ServiceError`；`ApiResponse`（**成功响应信封** `{code,message,data}`）**保留**——它是响应包装不是错误类型，§2.4 的 `ApiResponse::success()` 照常使用。
- domain 不定义错误（纯类型）。
- 两类错误的边界 = 两类 API 的边界，正好契合"代理 API 保持原生、管理 API 统一包装"。

### 2.4 分层前后对比

**当前**（Handler 里写 SQL）：
```rust
// api/handlers/admin/groups.rs — 612 行
pub async fn list(State(state): State<GroupState>, Query(query): Query<ListGroupsQuery>) {
    let mut builder = sqlx::QueryBuilder::new("SELECT id, name, provider, ... FROM groups");
    // ... 100+ 行 SQL 构造和映射
}
```

**重构后**（分层）：
```rust
// api/admin/routes.rs — ~30 行
pub async fn list(State(state): State<AppState>, Query(q): Query<RouteQuery>) {
    let routes = state.repositories.routes.list(q.into()).await?;
    Ok(Json(ApiResponse::success(routes)))
}

// repository/route_repository.rs — SQL 在这里
impl RouteRepository for SqliteRouteRepository {
    async fn list(&self, filter: RouteFilter) -> Result<Vec<Route>> { ... }
}

// domain/route.rs — 纯模型
pub struct Route {
    pub id: String,
    pub name: String,
    pub items: Vec<RouteItem>,
    // ...
}
```

---

## 3. 架构重构 + 插件系统

### 3.1 概览

| 工作项 | 优先级 |
|---|:---:|
| 分层重构（domain/repository/service + llm 引擎层） | P0 |
| 大文件拆分（所有 >500 行文件） | P0 |
| 分组 → 模型路由全链路重命名 | P0 |
| 插件系统（RequestPlugin 请求侧 + ResponsePlugin 响应侧 + 4 个内置插件） | P1 |
| stats.rs 职责拆分（统计 vs 预算） | P1 |

### 3.2 分组 → 模型路由重命名

全链路改名，不考虑兼容性：

| 位置 | 旧名 | 新名 |
|---|---|---|
| DB 表 | `groups` | `routes` |
| DB 表 | `group_items` | `route_items` |
| DB 列 | `group_id`（`route_items` / `usage_logs` / `usage_daily`） | `route_id` |
| DB 列 | `api_keys.allowed_groups` | `allowed_routes`（列内存的是 route id 列表，值不变，仅列名变） |
| DB 索引 | `idx_usage_logs_group_id` | `idx_usage_logs_route_id`（SQLite RENAME COLUMN 不自动改名，需重建） |
| Rust struct | `Group` / `GroupItem` | `Route` / `RouteItem` |
| Rust struct | `GroupState` 等 10 个 `*State` | 合并为单一 `AppState`（见 §5.1） |
| Rust 模块 | `api/handlers/admin/groups.rs` | `api/admin/routes.rs` |
| API 路径 | `/api/v1/admin/groups` | `/api/v1/admin/routes` |
| 前端路由 | `/groups` | `/routes` |
| 前端组件 | `GroupForm.tsx` | `RouteForm.tsx` |
| 侧边栏 | "分组管理" | "模型路由" |

> ⚠️ **`api_keys.allowed_groups` 容易漏**：它在 `8_add_api_key_allowed_groups.sql` 里以独立 `ALTER TABLE` 加入，不在 `1_initial_schema.sql`，全局搜 `group` 时若只看建表语句会错过。

### 3.3 插件系统设计

#### 3.3.1 架构

```
请求侧：请求进入 → 协议转换 → [请求插件链 RequestPlugin] → 发送上游
                              （cch / tracking / cache_key 改写请求体）

响应侧：上游响应 → [响应插件链 ResponsePlugin] → 转发客户端
                  （thinking：字段规范化 + 内容分离，流式逐块处理，§3.3.5）
```

#### 3.3.2 RequestPlugin / ResponsePlugin trait

请求侧（`RequestPlugin`）与响应侧（`ResponsePlugin`）分离：cch/tracking/cache_key 改请求体，thinking 改响应体，职责不混。

```rust
// llm/plugin/mod.rs

/// 插件执行上下文
pub struct PluginContext {
    pub upstream_endpoint: EndpointType,
    pub channel_id: String,
    pub host_key: String,       // 渠道 API Key 指纹（复用 usage_logs.upstream_key_hint，从 selection.channel 选中的 key 算）
    pub client_name: Option<String>, // User-Agent 中的客户端标识
}

/// 请求插件执行结果
pub enum PluginResult {
    Continue(serde_json::Value),   // 继续传递（可能已改写 body）
    Abort(ProxyError),             // 中止请求
}

/// 请求拦截改写（cch / tracking / cache_key）
#[async_trait::async_trait]
pub trait RequestPlugin: Send + Sync {
    fn id(&self) -> &'static str;
    fn matches(&self, ctx: &PluginContext) -> bool;
    /// setting 缺失时的默认开关（thinking→true，其余→false）
    fn default_enabled(&self) -> bool { false }
    async fn rewrite(&self, body: serde_json::Value, ctx: &PluginContext) -> PluginResult;
}

/// 响应改写（thinking）。非流式直接改 body；流式见 §3.3.5
#[async_trait::async_trait]
pub trait ResponsePlugin: Send + Sync {
    fn id(&self) -> &'static str;
    fn matches(&self, ctx: &PluginContext) -> bool;
    /// setting 缺失时的默认开关（thinking→true，其余→false）
    fn default_enabled(&self) -> bool { false }
    async fn rewrite_response(&self, body: serde_json::Value, ctx: &PluginContext) -> serde_json::Value;
}

/// 插件链：按顺序执行所有匹配且已启用的插件
pub struct PluginChain {
    request_plugins: Vec<Box<dyn RequestPlugin>>,
    response_plugins: Vec<Box<dyn ResponsePlugin>>,
    /// 各插件开关缓存（AppState 构造时从 settings 一次性 load，启动后必定 warm）
    enabled: Arc<RwLock<HashMap<&'static str, bool>>>,
    /// 全局总开关：false 时所有插件跳过（紧急回滚，plugin.master_switch）
    master_switch: Arc<AtomicBool>,
}

impl PluginChain {
    pub async fn apply_request(&self, mut body: Value, ctx: &PluginContext) -> Result<Value, ProxyError> {
        if !self.master_switch.load(Ordering::Relaxed) { return Ok(body); }
        let enabled = self.enabled.read().await;
        for p in &self.request_plugins {
            let on = *enabled.get(p.id()).unwrap_or(&p.default_enabled());
            if !p.matches(ctx) || !on { continue; }
            match p.rewrite(body, ctx).await {
                PluginResult::Continue(b) => body = b,
                PluginResult::Abort(e) => return Err(e),
            }
        }
        Ok(body)
    }
    pub async fn apply_response(&self, mut body: Value, ctx: &PluginContext) -> Value {
        if !self.master_switch.load(Ordering::Relaxed) { return body; }
        let enabled = self.enabled.read().await;
        for p in &self.response_plugins {
            let on = *enabled.get(p.id()).unwrap_or(&p.default_enabled());
            if !p.matches(ctx) || !on { continue; }
            body = p.rewrite_response(body, ctx).await;
        }
        body
    }
}
```

**开关机制（v1.1.3 落地）**：

- **冷启动**：AppState 构造时一次性从 settings 表 load 所有插件开关到 `enabled`，启动后缓存必定 warm；`unwrap_or` 用插件 `default_enabled()` 兜底（thinking→true，其余→false），避免 setting 缺失时 thinking 静默关闭。
- **不每请求查 DB**：`enabled` 是内存缓存，4 个插件不会变成每请求 4 次查询。
- **失效 hook（v1.1.3 核实项）**：settings 更新时需重建 `enabled` + `master_switch`。**实施时核实** settings 更新当前是否触发 ProxyCache 失效（约束 #6）——已覆盖则复用，未覆盖则新增 settings→PluginChain 重建 hook。
- **全局总开关 `plugin.master_switch`**：默认 true，置 false 时所有插件跳过（紧急回滚——cch/tracking/cache_key 默认对所有 Anthropic/Responses 流量改写，须有刹车）。

> ⚠️ **流式响应改写**：`thinking` 在流式下不能直接 `rewrite_response(body)`（body 是逐块到达的）。需在 `stream_executor` 加流式 hook，按 SSE 事件累积后再分离。复杂度高于请求侧，单列为 thinking 插件子任务（§3.3.5）。

#### 3.3.3 内置插件

**1. cch.rs — Claude Code 缓存标记清理**

借鉴 GT AI Gateway 的 `cchRewriter.ts`。

Claude Code 在 system prompt 中注入随机 `cch` 值（billing header），导致每次请求的 prompt hash 不同，上游缓存无法命中。清理后缓存命中率从 0% 提升到 97%。

```rust
// llm/plugin/cch.rs
pub struct CchRewriter;

#[async_trait]
impl RequestPlugin for CchRewriter {
    fn id(&self) -> &'static str { "cch_rewrite" }
    fn matches(&self, ctx: &PluginContext) -> bool {
        ctx.upstream_endpoint == EndpointType::Anthropic
    }
    async fn rewrite(&self, mut body: Value, _ctx: &PluginContext) -> PluginResult {
        // 清理 system 中的 x-anthropic-billing-header:cch=随机值 → cch=固定值
        // ...
        PluginResult::Continue(body)
    }
}
```

**2. tracking.rs — 隐私跟踪标记清洗**

借鉴 GT AI Gateway 的 `claudeCodeTrackingRewriter.ts`。

移除 Claude Code 注入的动态时间/地区/时区标记，防止被上游跟踪。

**3. cache_key.rs — prompt_cache_key 粘性路由注入**

借鉴 GT AI Gateway 的 `responsesPromptCacheKeyRewriter.ts`。

对 OpenAI Responses API 请求注入 `prompt_cache_key`，使无状态客户端也能获得粘性缓存效果。

**4. thinking.rs — 思维链处理（响应侧，v1.1 必做）**

实现为 `ResponsePlugin`（§3.3.2），作用于**响应体**，统一承接所有思维链相关处理。

> **决策：删除现有 reasoning 字段规范化逻辑，改由本插件统一实现。**
> 现有 `stream_executor.rs:445–775`（commit `4a3d250` "thinking_detected 统一覆盖 reasoning 字段"）在流式路径收集 `reasoning_content` 并写入 `reasoning` 字段——这段逻辑**移除**，职责并入 thinking 插件。避免两套思维链处理并存、边界模糊。

本插件职责（原"字段规范化" + "内容分离"合一）：

1. **字段规范化**：各厂商 `reasoning_content` / `thinking` / `<think>` 标记 → 统一写入响应 `reasoning` 字段（承接原 stream_executor 的活）。
2. **内容分离**：当思维链文本**意外混入正文 content block**（上游把 `<think>...</think>` 当普通 text 返回）时，从正文剥离、归入 thinking block。

实现要点：

- **流式必做**：原 reasoning 收集就是流式的（逐块 `reasoning_content` 累积），插件必须在流式 hook（§3.3.5）里逐块处理，不能只做非流式。
- **迁移即替换**：删除现有逻辑与插件上线必须在同一个 Phase，中间不能出现"reasoning 能力空窗"，否则是回归。配套测试要先覆盖原 reasoning 行为，再验证插件等价。
- 默认启用（`plugin.thinking_fix=true`）：它承接的是 v1.0 已有能力，不是可选增强。

#### 3.3.4 请求侧钩子注入点

在 `prepare.rs` 的 `prepare_proxy_request`（`prepare.rs:216`）中，**协议转换完成 + model override 之后**、构造 `PreparedProxyRequest` 返回前：

```rust
// prepare_proxy_request 内（prepare.rs:216）的现状：
//   234-242 RelayPipeline::prepare_request_async 完成协议转换 + target_model 覆盖（产出 Value）
//   243    serde_json::to_vec(&body)
//   247-251 from_slice → 注入 stream_options{include_usage:true} → to_vec   ← line 248 在此（非 model override）
// 插件链插在"协议转换 + model override 之后、stream_options 注入之前"，作用于最终上游格式的 body：
let ctx = PluginContext {
    upstream_endpoint: upstream_endpoint.clone(),
    channel_id: selection.channel.id.clone(),
    host_key: api_key_fingerprint,
    client_name: user_agent_parsed,
};
let rewritten = state.plugin_chain.apply_request(body_value, &ctx).await?;
// 之后再注入 stream_options（避免插件改写时把它改没），最后 to_vec
```

> ⚠️ 注意点：
> - **注入顺序**：插件在 model override **之后**（cch 清理针对最终上游格式）、stream_options 注入**之前**（改完 body 再补 stream_options，避免插件把它改没）。
> - **序列化合并**：prepare.rs 现状已有两轮 `to_vec`（243 转 body + 248 stream_options 重写），加插件会变第三轮。重构时改为单次 Value 流转：`prepare_request_async 产出 Value → 插件改写 → 注入 stream_options → 一次 to_vec`。
> - **非 JSON body**：images/embeddings 等路径若 body 非 JSON，`from_slice::<Value>` 会失败。插件链只在"body 解析成功 + endpoint 匹配"时介入（双重门控）。

#### 3.3.5 响应侧钩子注入点

- **非流式**：`llm/relay/executor.rs` 拿到上游 JSON 响应后，`apply_response` 改写，再提取 usage / 落库。
- **流式**：`llm/relay/stream_executor.rs` 在**转发管道内逐块处理**（SSE 事件解析后，复用现有 reasoning 累积位置改造）——不依赖 §4.2 的 `StreamAccumulator`（那是"累积原始字节用于落库"，thinking 是"解析后改写转发流"，两回事）。thinking 插件自行维护 reasoning 字段 + content block 的结构化累积。

> 注：thinking 流式与 §4.2 响应记录是**独立的两条流式旁路**——一个改转发内容，一个存日志。两者都在 stream_executor，但互不依赖，可分别落地。

### 3.4 大文件拆分计划

#### metrics/query/mod.rs (1202 行) → 拆分

```
repository/
├── usage_repository/
│   ├── mod.rs          # UsageRepository trait + 通用类型（StatsOverview, PagedResult）
│   ├── overview.rs     # get_overview（总/今日统计）
│   ├── trend.rs        # get_daily_stats / get_daily_stats_by_range（日/小时趋势）
│   ├── model.rs        # get_model_stats / by_range（按模型聚合）
│   ├── channel.rs      # get_channel_stats / by_range（按渠道聚合）
│   ├── api_key.rs      # get_api_key_stats（按 Key 聚合）
│   ├── logs.rs         # get_logs / get_log_detail / get_log_models（日志查询）
│   └── latency.rs      # get_latency_percentiles（延迟百分位）
```

#### llm/relay/stream_executor.rs (1072 行) → 拆分 + 删 reasoning

> 拆分时**同步删除 §3.3.3 移除的 reasoning 收集段**（现 445–775），该职责迁到 thinking 插件。拆分与删除同 Phase，避免 reasoning 能力空窗。

```
llm/relay/
├── stream_executor.rs  # 核心执行流程（<400行）
├── stream_parser.rs    # SSE 事件解析 + usage 提取（从 stream_executor 拆出）
├── stream_error.rs     # SSE 错误归因 + status 码修正（从 stream_executor 拆出）
```

#### api/handlers/admin/groups.rs (612 行) → 拆分

```
api/admin/routes.rs     # Handler（~100行，只调 repository）
repository/route_repository.rs # SQL CRUD（~300行）
domain/route.rs         # 模型定义（~50行）
```

#### stats.rs (314 行) → 拆分

```
api/admin/stats.rs      # 统计查询 handler（~150行）
api/admin/budget.rs     # 预算管理 handler（~100行）
```

#### llm/protocol/inbound/*.rs → 拆分

inbound 只处理**客户端请求**（入站解析），按请求协议（chat / responses / messages）或职责（字段映射 / tool call 状态机 / 流式 delta 累积）拆。**响应解析不属 inbound**（在 outbound / sse.rs），勿按"请求/响应/流式"切。每个 >500 行的入站文件按此维度拆。

#### metrics/ 模块完整去向（勿遗漏）

`metrics/` 共 9 个文件，前文只点名了 query/recorder/model，其余去向要明确，否则拆分会留孤儿：

| 现文件 | 去向 |
|---|---|
| `metrics/query/mod.rs` (1202) | `repository/usage_repository/`（上文） |
| `metrics/recorder/mod.rs` (639) | `service/stats/recorder.rs` |
| `metrics/recorder/redaction.rs` | ⚠️ **保留并明确归属**——脱敏逻辑，与 §4.2 响应记录、§4.4 门户隐私直接相关。建议落 `service/stats/redaction.rs`，响应落库前先过脱敏 |
| `metrics/model.rs` (694) | `service/pricing/` |
| `metrics/pricing.rs` | `service/pricing/`（与 model.rs 合并） |
| `metrics/attempt.rs` | `domain/usage.rs`（AttemptStats 是纯数据）或 `llm/relay/`（若含行为） |
| `metrics/usage/estimator.rs` | `service/pricing/estimator.rs`（TokenEstimator 纯计算，prepare.rs 在用） |
| `metrics/usage/mod.rs` | 视内容并入上述 |
| `metrics/mod.rs` | 拆解后删除 |

> ⚠️ **redaction 必须显式安排**：§4.2 要默认记录 response_content，§4.4 要保护门户隐私。脱敏（密钥 / PII / 敏感 header）是这两条的安全前提，不能在拆分中丢弃或漏迁。

#### 其他 >500 行文件拆分（v1.1.4）

下列文件 >500 行但上文未单独列出，v1.1.4 按维度拆（目标 <500 行，函数级划分实施时定）：

| 文件 (行数) | 拆分维度 |
|---|---|
| `llm/relay/executor.rs` (720) | 流程编排 / usage 提取 / 错误归因（对齐 stream_executor 拆法） |
| `llm/relay/pipeline.rs` (696) | 按管道阶段拆 |
| `llm/protocol/sse.rs` (634) | 事件解析 / 错误提取 / 完成检测 |
| `llm/relay/prepare.rs` (549) | 已与插件钩子强绑（§3.3.4），轻拆或保留 |
| `llm/scheduler/state.rs` (539) | 状态结构 / 评分 / 候选构建 |

### 3.5 DB 迁移

```sql
-- src/db/migrations/17_rename_groups_to_routes.sql（v1.1.1 批次，路径对齐约束 #5，migrations 不随重构迁移）

-- 表改名
ALTER TABLE groups RENAME TO routes;
ALTER TABLE group_items RENAME TO route_items;

-- 列改名（先改表名再改列名）
ALTER TABLE route_items RENAME COLUMN group_id TO route_id;
ALTER TABLE usage_logs   RENAME COLUMN group_id TO route_id;
ALTER TABLE usage_daily  RENAME COLUMN group_id TO route_id;
-- ⚠️ 容易漏：allowed_groups 以独立 ALTER 加入，不在 initial_schema
ALTER TABLE api_keys     RENAME COLUMN allowed_groups TO allowed_routes;

-- 索引重建：SQLite RENAME COLUMN 不会自动重命名索引，旧索引名会名不副实
DROP INDEX IF EXISTS idx_usage_logs_group_id;
CREATE INDEX IF NOT EXISTS idx_usage_logs_route_id ON usage_logs(route_id);

-- 注：UNIQUE 约束（route_items 的 UNIQUE(group_id,channel_id,model_name)、
-- usage_daily 的 UNIQUE(date,api_key_id,channel_id,group_id,model)）随列改名自动跟随，无需重建。

-- 新增插件配置项（随 v1.1.1 迁移先建，v1.1.3 插件系统启用时读取；v1.1.1/3 期间存在但闲置）
INSERT OR IGNORE INTO settings (key, category, value, description) VALUES
    ('plugin.cch_rewrite', 'plugin', 'true', '清理 Claude Code cch 标记，提升缓存命中率'),
    ('plugin.tracking_removal', 'plugin', 'true', '清洗 Claude Code 隐私跟踪标记'),
    ('plugin.cache_key_injection', 'plugin', 'true', '注入 prompt_cache_key 实现粘性路由'),
    ('plugin.thinking_fix', 'plugin', 'true', '思维链处理：字段规范化 + 内容分离（承接 v1.0 reasoning 能力，默认开）'),
    ('plugin.master_switch', 'plugin', 'true', '全局总开关：false 时所有插件跳过（紧急回滚）');
```

---

## 4. 统计增强 + Key 门户

### 4.1 概览

| 工作项 | 优先级 |
|---|:---:|
| 响应内容记录（流式路径填充 response_content） | P0 |
| 请求详情可视化（prompt 全文 / 工具调用链 / 耗时时间线） | P0 |
| Key 持有者门户（`/portal`，key 换短期 token 鉴权） | P0 |
| 统计扩展（缓存命中分析 / 错误率趋势 / 自定义日期范围） | P1 |
| 供应商预设 URL | P2 |

### 4.2 响应内容记录

#### 4.2.1 现状

`usage_logs.response_content` 字段已存在（DB schema 有列），但：
- 非流式路径：未填充（`save_request_record` 中传 None）
- 流式路径：未填充（SSE 块未被 accumulate）

#### 4.2.2 方案

**非流式**：`llm/relay/executor.rs` 拿到上游响应后，直接传入 `response_content`。

**流式**：借鉴 GT AI Gateway 的 accumulator 模式：

```rust
// llm/relay/stream_accumulator.rs

/// SSE 块累积器：在转发流式响应给客户端的同时，累积完整内容用于日志
pub struct StreamAccumulator {
    blocks: Vec<u8>,
    max_size: usize,  // 限制内存，默认 1MB
}

impl StreamAccumulator {
    pub fn push(&mut self, chunk: &[u8]) {
        if self.blocks.len() + chunk.len() <= self.max_size {
            self.blocks.extend_from_slice(chunk);
        }
    }
    pub fn finish(self) -> Option<String> {
        if self.blocks.is_empty() { return None; }
        String::from_utf8(self.blocks).ok()
    }
}
```

在 `stream_executor` 的流式转发管道中插入 accumulator，流结束后将累积内容写入 `response_content`。

**存储策略**（参考 GT AI Gateway）：

- **两个上限要分清**：`StreamAccumulator.max_size = 1MB` 是**流式内存累积**上限（防 OOM）；落库时 `response_content` **截断到 100KB**。两者不矛盾，是不同阶段的不同阈值，文档原文并列易误读。
- **默认开启，可配置关闭**（新增 setting `usage.record_content`）。
- **拆表存储（已定）**：大 TEXT 直接塞 `usage_logs` 会拖慢其 9+ 处分页/聚合查询——SQLite 单文件库对大行更敏感。新建 `usage_payloads`，`usage_logs` 只留统计字段，详情按需 join：

  ```sql
  -- src/db/migrations/18_create_usage_payloads.sql（v1.1.5 批次，路径对齐约束 #5）
  CREATE TABLE IF NOT EXISTS usage_payloads (
      log_id TEXT PRIMARY KEY REFERENCES usage_logs(id) ON DELETE CASCADE,
      request_content TEXT,
      response_content TEXT,
      created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
  );
  CREATE INDEX IF NOT EXISTS idx_usage_payloads_created_at ON usage_payloads(created_at);

  -- 记录开关（默认开）
  INSERT OR IGNORE INTO settings (key, category, value, description) VALUES
      ('usage.record_content', 'usage', 'true', '是否记录请求/响应原文到 usage_payloads（关闭=新请求不写 payload，历史不受影响）');
  ```
  ```sql
  -- src/db/migrations/19_drop_usage_logs_content_columns.sql（v1.1.5 批次）
  -- 回填：先把 usage_logs 现有内容搬到 usage_payloads
  INSERT OR IGNORE INTO usage_payloads (log_id, request_content, response_content, created_at)
  SELECT id, request_content, response_content, created_at FROM usage_logs
  WHERE request_content IS NOT NULL OR response_content IS NOT NULL;
  -- 再从 usage_logs 删除这两列（SQLite 3.35.0+ 支持 DROP COLUMN；更低版本需"重建表"套路）
  ALTER TABLE usage_logs DROP COLUMN request_content;
  ALTER TABLE usage_logs DROP COLUMN response_content;
  ```
- **落库前过 redaction**（见 §3.4）：密钥、敏感 header 先脱敏（防 key 泄露到日志）；**prompt 正文默认可见**（调试优先，已定），如后续需对某些 Key 的正文脱敏，再加按 Key 开关。
- 提供 API 清理历史 payload（保留统计摘要，删除内容），并评估 SQLite `VACUUM` 策略。

### 4.3 请求详情可视化

#### 4.3.1 数据基础

详情接口按 `log_id JOIN usage_payloads`（§4.2 拆表后，原文在这张表）取：
- `request_content`（请求 JSON 全文）
- `response_content`（响应 JSON 全文）

> `usage_payloads` 行可能不存在（关闭 `usage.record_content` 或被清理 API 删过），详情接口需对这两字段返回 null。下列字段仍在 `usage_logs`：
- `attempts`（多渠道尝试链）
- `latency_ms` / `ttft_ms`（延迟数据）
- `input_tokens` / `output_tokens` / `cache_read` / `cache_creation`

#### 4.3.2 前端展示

请求日志列表页 → 点击某条 → 展开详情抽屉：

```
┌─────────────────────────────────────────────────┐
│ 请求详情 #xxx                                    │
├─────────────────────────────────────────────────┤
│                                                 │
│ [概览] 模型: claude-sonnet-4-20250514           │
│        渠道: Anthropic Official                  │
│        状态: 200 ✓    耗时: 1.2s                │
│        TTFT: 320ms     类型: 协议转换            │
│                                                 │
│ [Token 构成]                                    │
│   Input: 12,450  Output: 850                    │
│   Cache Read: 10,200  Cache Creation: 2,250     │
│   (缓存命中率: 82%)                              │
│                                                 │
│ [耗时时间线]                                     │
│   T=0ms ─── 请求到达                             │
│   T=45ms ── 渠道选择完成                         │
│   T=320ms ─ 首个 Token (TTFT)                   │
│   T=1200ms ─ 完成                               │
│                                                 │
│ [多渠道尝试] (如果有重试)                        │
│   ✓ Channel A (claude-sonnet)  1.2s  200       │
│   ✗ Channel B (openai-proxy)   0.3s  429       │
│                                                 │
│ [请求内容]  ▼ 展开                               │
│   system: "You are a helpful assistant..."      │
│   messages: [...]                               │
│   tools: [...]                                  │
│                                                 │
│ [响应内容]  ▼ 展开                               │
│   content[0]: { type: "thinking", ... }         │
│   content[1]: { type: "text", text: "..." }     │
│                                                 │
└─────────────────────────────────────────────────┘
```

**JSON 渲染**：使用 `react-json-view-lite` 或类似库，支持折叠/展开/高亮。

### 4.4 Key 持有者门户

#### 4.4.1 设计

```
访问方式: http://localhost:8080/portal?key=sk-gr-xxxxxxxx（仅入口，换完即 302 到 /portal）
认证方式: key 校验通过后签发 JWT（payload: api_key_id + 10 分钟 exp，httpOnly cookie 下发），后续 /api/v1/portal/* 验 JWT；过期带 key 重新换发。无状态、免新增 session 表（符合 §4.4.4）
无需登录/密码
```

> **决策：key 不长期留在 URL。** sk-xxx 进 query string 会被浏览器历史 / Referer / access log / 反代日志记录（OWASP 反模式）。流程：`/portal?key=xxx`（入口）→ 后端校验 key → 下发 10 分钟 httpOnly cookie → 302 到 `/portal`（不带 key）→ 后续 API 用 cookie 中的 token 鉴权。

#### 4.4.2 页面内容

```
┌─────────────────────────────────────────────────┐
│ Galaxy Router · Key 用量                        │
│                                                 │
│ [预算]                                          │
│   月度: $12.50 / $50.00 (25%)                   │
│   日度: $1.20 / $5.00 (24%)                     │
│                                                 │
│ [今日趋势]                                      │
│   请求: 42    Token: 125K in / 8K out           │
│   成功率: 95%                                   │
│                                                 │
│ [7天趋势图]                                     │
│   ▁▃▅▇▅▃▂ (每日请求量)                         │
│                                                 │
│ [最近请求] (简表，不含 prompt 全文)              │
│   时间     模型          Token   耗时   状态    │
│   10:32    claude-sonnet 12K/850 1.2s   ✓      │
│   10:28    gpt-4o        8K/600  0.8s   ✓      │
│   10:15    claude-sonnet 15K/2K  1.5s   ✗ 500  │
│                                                 │
└─────────────────────────────────────────────────┘
```

#### 4.4.3 后端 API

```
# 鉴权统一走 cookie 中的 JWT（§4.4.1 签发），不在 URL 带 key
GET /api/v1/portal/overview      → 预算+今日概览
GET /api/v1/portal/trend?days=7  → N天趋势
GET /api/v1/portal/logs?page=1   → 最近请求列表
```

安全约束：
- portal API 只返回该 key 自己的数据（WHERE api_key_id = ?）
- 不返回 request_content / response_content（portal 是 **Key 持有者视角**，不看 prompt 全文；管理员走管理后台，§4.2 默认可见原文——两者角色不同）
- **401 vs 200 是 status oracle**：仅"不泄露 key 是否存在"不够。所有 portal API 对未授权请求返回**不可区分**的响应（相同状态码 + 相近耗时），叠加 **IP 维度限流**防暴力枚举。
- token 鉴权失败 / 过期 / key 失效 三种情况返回同一错误，不泄露具体原因。

#### 4.4.4 数据模型

复用现有 `api_keys` + `budget_limits` + `usage_logs` 三表，不需要新增用户表。

"单管理员、多用户"模式：
- 管理员创建 API Key，分发给团队成员
- 每个 Key 有独立的预算限额（已有 `budget_limits`）
- Key 持有者通过 `/portal?key=xxx` 查看自己的用量
- 管理员在管理后台看到所有 Key 的聚合统计

### 4.5 统计与渠道增强

#### 4.5.1 缓存命中分析

```sql
-- 缓存命中率趋势
SELECT
    date(datetime(created_at, ?)) as date,
    COUNT(*) as total,
    SUM(CASE WHEN cache_read_tokens > 0 THEN 1 ELSE 0 END) as cache_hit,
    COALESCE(SUM(cache_read_tokens), 0) as cache_read_total,
    COALESCE(SUM(input_tokens), 0) as input_total
FROM usage_logs
WHERE created_at >= ? AND created_at < ?
GROUP BY date
```

在概览页增加"缓存命中率"指标卡片。

#### 4.5.2 错误率趋势

在每日趋势中增加错误率折线图，按 status_code 分类（4xx / 5xx / 超时）。

#### 4.5.3 供应商预设

> **命名决策**：channel（渠道）实例保留，provider 仅作**预设模板**（辅助快速创建渠道）。对齐 octopus（`providers.json` 模板 + `Channel` 实例）。galaxy 一个供应商可对应多个渠道（多 key 分摊 / 官方+中转 / 多地域），调度器按 `priority/weight` 在多渠道间负载均衡——故 channel 不改名。

内置供应商预设，创建渠道时选供应商 → 一键填充 `endpoints` 数组。**结构对齐 `EndpointConfig`（`{type, base_url, enabled, headers}`）和 octopus `providers.json`**：一个供应商可挂多个 endpoint（很多渠道同时提供 OpenAI 兼容 + Anthropic 兼容两种接口）。

```json
// src/infra/presets.json
// type 取值对齐 EndpointType serde 序列化（rename_all="snake_case"）：
//   openai_chat / openai_response / anthropic / gemini / openai_embedding / openai_images
[
  { "name": "OpenAI", "endpoints": [
      { "type": "openai_chat", "base_url": "https://api.openai.com" } ] },
  { "name": "Anthropic", "endpoints": [
      { "type": "anthropic", "base_url": "https://api.anthropic.com" } ] },
  { "name": "Google Gemini", "endpoints": [
      { "type": "gemini", "base_url": "https://generativelanguage.googleapis.com" } ] },
  { "name": "DeepSeek", "endpoints": [
      { "type": "openai_chat", "base_url": "https://api.deepseek.com" } ] },
  { "name": "智谱 GLM", "endpoints": [
      { "type": "openai_chat", "base_url": "https://open.bigmodel.cn/api/paas/v4" },
      { "type": "anthropic",   "base_url": "https://open.bigmodel.cn/api/anthropic" } ] },
  { "name": "月之暗面 Kimi", "endpoints": [
      { "type": "openai_chat", "base_url": "https://api.moonshot.cn" } ] },
  { "name": "阿里百炼", "endpoints": [
      { "type": "openai_chat", "base_url": "https://dashscope.aliyuncs.com/compatible-mode/v1" },
      { "type": "anthropic",   "base_url": "https://dashscope.aliyuncs.com/compatible-mode" } ] },
  { "name": "MiniMax", "endpoints": [
      { "type": "openai_chat", "base_url": "https://api.minimaxi.com/v1" },
      { "type": "anthropic",   "base_url": "https://api.minimaxi.com/anthropic" } ] },
  { "name": "通用自定义", "endpoints": [
      { "type": "openai_chat", "base_url": "" },
      { "type": "anthropic",   "base_url": "" } ] }
]
```

> 参考自 octopus `internal/assets/providers.json`（同款"一个供应商多 endpoint"结构）。智谱 / 百炼 / MiniMax 都同时提供 OpenAI 兼容和 Anthropic 兼容两套接口，预设里都列出，创建渠道时勾选启用的 endpoint。base_url 路径前缀（`/v1` `/v1beta` 等）实现时按 outbound 构造逻辑校准。

---

## 5. 实施计划

### 5.1 发布批次

v1.1 是统一版本线（架构重构 + 功能增强全部归此），通过 patch 版本分批次发布，每批次可独立编译/测试/发布。按依赖顺序排列：

| 批次 | 内容 | 依赖 | DB 迁移 |
|---|---|---|---|
| **v1.1.0** | 架构骨架：`domain/` `repository/` `service/` `llm/` 目录 + `AppState` 空壳（不影响现有功能） | — | — |
| **v1.1.1** | 重命名 Group→Route 全链路（DB + Rust + API + 前端） | v1.1.0 | 17 |
| **v1.1.2** | SQL 下沉 handler→repository + 各 handler 切换 `State<AppState>` | v1.1.1 | — |
| **v1.1.3** | 插件系统：RequestPlugin/ResponsePlugin + 4 插件（thinking 删旧 reasoning，同 commit） | v1.1.2 | — |
| **v1.1.4** | 大文件拆分（stream_executor / executor / pipeline / sse / prepare / scheduler-state / inbound） | v1.1.3 | — |
| **v1.1.5** | 响应内容记录：StreamAccumulator + usage_payloads + redaction 接入 | v1.1.4 | 18/19 |
| **v1.1.6** | 请求详情可视化（前端详情抽屉，消费 usage_payloads） | v1.1.5 | — |
| **v1.1.7** | Key 持有者门户（portal + JWT 鉴权） | v1.1.2 | — |
| **v1.1.8** | 统计与渠道增强（缓存命中分析 / 错误率趋势 / 供应商预设） | v1.1.2 | — |

> 各批次完成后跑 `make check` + 相关测试，绿了再发下一个 patch。前端适配随对应批次落地（v1.1.1 改名、v1.1.6 可视化、v1.1.7 门户）。

### 5.2 关键约束

> ⚠️ **v1.1.1 重命名是单个大 commit，不满足"每步编译通过"**：Rust struct/字段改名是原子的，`Group` 引用散落在 llm/relay/scheduler/api/metrics，改名期间任何中间状态都编译不过。v1.1.1 内部按"DB 迁移 → 后端全量改名（一个大 commit，CI 全绿）→ 前端改名"切分，不按 Rust 模块逐个改。其余批次（v1.1.2 起）恢复"每步可编译"。

**State 合并（v1.1.0 引入 / v1.1.2 切换）**：现有 10 个 admin handler `*State` 字段高度重叠（`pool` 重复 7 次，`cache` / `timezone_offset` / `http_client` 各重复多次），分 State 是纯样板。对齐 axonhub（单一 `Server` struct + `dependencies/`），合并为单一 `AppState`——持有 `Repositories` + `Services` + `ProxyCache` + `http_client` + `api_key_cache` + `config` + `timezone_offset` 等全部依赖，所有 handler 统一 `State<AppState>`。

**thinking 删 reasoning（v1.1.3）**：删除 `stream_executor.rs:445–775` 与 thinking 插件上线**同 commit**，避免 reasoning 能力空窗或双写。先补齐插件测试覆盖原 reasoning 行为再切换。

**响应记录迁移（v1.1.5）**：迁移 18 建 `usage_payloads` + 回填 + `usage.record_content` 设置；迁移 19 从 `usage_logs` 删 content 列（SQLite 3.35+ DROP COLUMN）。落库前过 redaction。

---

## 6. 风险与缓解

| 风险 | 影响 | 缓解 |
|---|---|---|
| 大规模重构引入回归 bug | 高 | 每个 Phase 完成后运行全量测试（`make check`），保持测试覆盖 |
| 分组重命名遗漏（尤其 `api_keys.allowed_groups`） | 中 | 全局搜 `group`，覆盖 DB 列/索引/Rust/前端；`allowed_groups` 在独立 ALTER 里易漏，见 §3.2 |
| 流式 response_content 累积增加内存 | 中 | accumulator 内存上限 1MB；落库截断 100KB；拆 `usage_payloads` 表避免拖慢 usage_logs（§4.2） |
| 插件改写破坏请求格式 | 高 | 每个插件单测验证改写后 JSON 合法；非 JSON body 路径跳过（§3.3.4） |
| 插件开关每请求查 DB 拖慢转发 | 高 | 开关进内存缓存，随 settings 变更失效（§3.3.2，约束 #6） |
| Key 门户 `sk-xxx` 在 URL 暴露 | 高 | key 仅换短期 token，后续走 token；IP 限流 + 统一错误响应（§4.4） |
| v1.1.1 改名期间编译不过 | 中 | 接受为单个大 commit，不强行分小步（§5.2） |
| 删除现有 reasoning 逻辑迁移到 thinking 插件期间出现能力空窗 | 高 | 同 Phase 删除 + 上线，先补齐插件测试覆盖原 reasoning 行为再切换（§3.3.3） |
| redaction 在 metrics 拆分中漏迁 | 高 | 显式安排去向，响应落库前必须过脱敏（§3.4 / §4.2） |

---

## 7. 决策清单（Review）

### 已确认

| 决策点 | 结论 | 位置 |
|---|---|---|
| LLM 引擎层组织 | relay/protocol/scheduler/**plugin 聚合到 `llm/`**（对齐 axonhub；三模块只服务 relay，聚合后依赖最内聚） | §2.2 / §2.3 |
| 错误类型分层 | **两层**：`ProxyError`（原生透传）+ `ServiceError`（统一封装），不设 RepoError | §2.3.1 |
| State 合并 | **全合并为单一 `AppState`**（字段重叠严重，`pool` 重复 7 次；对齐 axonhub） | §5.1 |
| thinking 插件 | **v1.1 必做**；删除现有 reasoning 字段规范化，由插件统一承接 | §3.3.3 |
| 供应商预设结构 | 对齐 octopus `providers.json`，**一个供应商多 endpoint**（智谱/百炼/MiniMax 均双接口） | §4.5.3 |
| 渠道命名 | **channel（渠道）保留，provider 仅作预设模板**；galaxy 一个供应商可对应多个渠道（多 key 分摊 / 主备 / 多地域），调度器按 `priority/weight` 负载均衡，故不改名 | §4.5 |
| Key 门户认证 | **key 换 JWT**（无状态，httpOnly cookie，免新增 session 表），key 仅作入口不长期留 URL | §4.4.1 |
| response_content 存储 | **拆 `usage_payloads` 表**（log_id, request_content, response_content），`usage_logs` 只留统计字段（SQLite 单文件库对大行敏感） | §4.2 |
| 管理员可见原文 | **默认可见**（调试优先）；密钥/敏感 header 仍落库前 redaction | §4.2 |

> **v1.1 版本线 + 9 批次（§5.1）已定，可进入 v1.1.0 实施。** 指纹算法 / JWT / 灰度开关等实施细节见各章节，实施时若微调回此处更新。

---

## 附录 A：参考项目设计借鉴

### GT AI Gateway

| 设计 | 借鉴到 | 方式 |
|---|---|---|
| cch 缓存改写（0%→97%命中率） | plugin/cch.rs | 直接移植逻辑到 Rust |
| 隐私跟踪标记清洗 | plugin/tracking.rs | 直接移植正则逻辑 |
| prompt_cache_key 粘性路由 | plugin/cache_key.rs | 直接移植注入逻辑 |
| StreamAccumulator 模式 | stream_accumulator.rs | 借鉴流式累积思路 |
| 请求内容可视化分析 | 请求详情抽屉 | 借鉴前端展示设计 |

### Octopus

| 设计 | 借鉴到 | 方式 |
|---|---|---|
| 多 Key 分发模式 | Key 门户 | 确认"单管理员多用户"模式可行 |
| 按 Key 查看用量 | portal API | 借鉴用量查询维度 |
| 供应商预设（多 endpoint 模板） | presets.json | 直接借鉴 providers.json 结构（§4.5.3） |
