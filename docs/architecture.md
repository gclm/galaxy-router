# Galaxy Router 架构基准（Architecture Baseline）

> 日期：2026-07-15
> 状态：**生效中（权威基准）**
> 范围：分层架构、目录结构、层间边界、依赖方向

## 0. 文档定位

本文件是 galaxy-router 的**架构单一真相源**。所有结构调整、代码归属决策、重构批次都以本文件为准。

与既有文档的关系：

- **本文件**：管"怎么搭"——分层、目录、边界、依赖。
- `docs/design-v1-refactor.md`：管"做什么"——v1.1 功能路线图（插件系统、统计增强、Key 门户等）。
- 两者互补。`design-v1-refactor.md §2` 的架构部分若与本文件冲突，**以本文件为准**（本文件是其后制定的、更完整的基准）。

当代码与本文件不一致时：先判断是"待消除的债"（参照 §8）还是"文档过时"——前者改代码，后者更新本文。

---

## 1. 架构选型

### 1.1 选定方案：务实三层 + Repository

```
api（Handler） → service（业务逻辑） → repository（数据访问） → domain（领域模型）
                                    ↘
                                     infra（技术设施：db / cache / config / 外部客户端）
```

Controller → Service → Repository → Domain 的经典务实分层，匹配 galaxy 作为**数据管道 + 配置管理**的应用本质。

### 1.2 否决方案及理由

| 方案 | 否决理由 |
|---|---|
| **DDD**（聚合/值对象/领域事件） | 过重。galaxy 核心是请求转发管道，没有复杂业务不变量与领域事件；硬上会造空壳（当前的 `Services` 空壳即 DDD 思维残留） |
| **六边形 / 端口适配器** | galaxy 单数据库（SQLite，约束锁定），无多源切换与 mock 单测需求，全套 adapter 收益≈0、成本实打实 |
| **洋葱 / Clean Architecture** | 层太多，中等项目过度 |
| **MVC** | UI 范式（Model-View-Controller）。galaxy 前端是独立 React SPA，后端纯 JSON API **没有 View**；且后端 MVC 的 Model 层内部必然再分 entity/dao/service，等于把已分好的层合并再拆 |

### 1.3 设计哲学

- **务实分层，不过度**：层够用即可，不为对称/教条增加空层。
- **SQL 只在 repository**：这是分层重构的头号目标（galaxy v1.0.3 的核心病根就是 SQL 散落在 handler/state）。
- **判据优于死规则**：用"换业务实体还在不在""有没有编排逻辑"这类判据决定归属（见 §5），而不是机械地"每实体建一个 service"。
- **演进留口**：选务实薄分层，但当多数据库/mock 单测需求出现时，可升级到严格 clean（见 §9.3）。

---

## 2. 分层模型与依赖方向

### 2.1 依赖方向规则（不可违反）

```
            ┌─→ llm（relay 编排 → protocol / scheduler / plugin）─→ infra(reqwest)
api  ───────┤
            └─→ service ──→ repository ──→ domain
                                          infra（repository 用 infra 的连接池）
```

- 依赖**只能向内/向下**：api → service → repository → domain。
- **domain 零框架依赖**：允许 `serde`（数据形状描述），禁止 `axum/sqlx/reqwest`。
- **repository 依赖 infra 提供的连接池**，但 repository 自身（trait+impl）放 `repository/`，不进 infra（见 §4.3 与 §5.1）。
- **llm 与 service 平级**：转发引擎聚合层，依赖 infra。relay→protocol/scheduler/plugin 是引擎内部调用。
- **禁止反向依赖**：domain/repository/service 不得 import api；infra 不得 import llm/service。

### 2.2 顶层模块速查

| 模块 | 角色 | 一句话职责 |
|---|---|---|
| `api/` | 接入层 | HTTP 适配：解析请求、调 service/llm、格式化响应 |
| `service/` | 业务逻辑层 | 业务规则、事务编排、多步工作流（按业务能力组织） |
| `repository/` | 数据访问层 | 业务实体的存取，**所有 SQL 收在此**（trait + impl 同放） |
| `domain/` | 领域层 | 实体 + 纯数据形状 + 不依赖框架的业务规则 |
| `infra/` | 基础设施层 | 技术设施：db 连接池/迁移、cache、config、外部客户端 |
| `llm/` | 转发引擎层 | relay/protocol/scheduler/plugin 聚合，请求转发管道 |

---

## 3. 目标目录结构

```
src/
├── api/                     # 接入层（axum）
│   ├── handlers/
│   │   ├── admin/           #   管理 API（/api/v1/admin/*，薄 handler）
│   │   └── proxy/           #   代理 API（/v1/*，原生协议透传）
│   ├── middleware/          #   中间件（鉴权、录制注入等）
│   └── router.rs            #   路由注册
│
├── service/                 # 业务逻辑层（按业务能力组织，非按实体）
│   ├── pricing/             #   定价：ModelRegistry + 远程刷新 + 价格计算
│   ├── stats/               #   统计：recorder（记录）+ query（聚合查询）
│   ├── channel/             #   渠道：CRUD 业务规则 + 带缓存的渠道查询
│   ├── route/               #   路由：CRUD + 带缓存的路由匹配
│   ├── api_key/             #   Key + 鉴权 + 预算业务规则
│   ├── settings/            #   设置管理
│   ├── backup/              #   导入/导出/重置事务编排
│   └── update_check/        #   版本检查（UpdateCheckContext）
│
├── repository/              # 数据访问层（trait + impl 同文件，务实派）
│   ├── mod.rs               #   Repositories（统一持有所有 repository）
│   ├── channel_repository.rs
│   ├── route_repository.rs
│   ├── api_key_repository.rs
│   ├── budget_repository.rs
│   ├── auth_repository.rs
│   ├── settings_repository.rs
│   ├── system_info_repository.rs
│   └── usage_repository/    #   统计存取（日志 + payload，可拆子目录）
│
├── domain/                  # 领域层（纯 struct/trait，零框架依赖）
│   ├── channel.rs           #   Channel / EndpointConfig / UpstreamApiKey
│   ├── route.rs             #   Route / RouteItem
│   ├── usage.rs             #   UsageLog / RequestRecord / AttemptStats
│   └── ...
│
├── infra/                   # 基础设施层（纯技术设施，不认业务实体）
│   ├── db/                  #   连接池、迁移执行器（SQL 文件留 src/db/migrations）
│   ├── cache.rs             #   ProxyCache（通用缓存机制）
│   └── config.rs            #   配置加载
│
├── llm/                     # 转发引擎聚合层
│   ├── relay/               #   转发管道（编排者）
│   ├── protocol/            #   协议转换（inbound / outbound / sse）
│   ├── scheduler/           #   负载均衡（选渠道）
│   └── plugin/              #   请求/响应拦截改写
│
├── error/                   # 两类错误：proxy（/v1/*） / service（/api/v1/admin/*）
├── util/                    # 纯函数工具（generate_id 等）
└── main.rs                  # 入口：组装各层依赖
```

> `auth/`（JwtService）、`static_assets/`（前端静态资源）作为辅助顶层模块保留，不属于核心分层链路。

---

## 4. 各层职责详解

### 4.1 api（接入层）

**职责**：HTTP 边界适配——解析请求参数、调用 service/llm、把结果格式化成响应。**薄 handler**。

**放什么**：参数校验（格式层面）、调用 service/llm、响应包装（`ApiResponse` 或原生透传）、路由注册。

**不放什么**：业务规则、SQL、事务编排、跨 repository 的逻辑。一个 handler 写超过 ~50 行业务逻辑，通常是该下沉到 service 的信号。

**基准调用链**：`handler → service → repository`。例外：零业务逻辑的纯透传查询（无校验/无编排/无副作用）可 `handler → repository`，但应少用、可解释。

### 4.2 service（业务逻辑层）

**职责**：业务规则、不变量、事务编排、多步工作流、副作用协调（缓存失效等）。**业务逻辑的家**。

**组织方式**：**按业务能力组织，不是按实体**。一个业务能力（如"渠道管理"含 CRUD + 探测 + 缓存查询）是一个 service 模块。

**形态**（Rust 实践）：
- 无状态用例 → **普通函数模块**。
- 有状态 / 需共享依赖（持有 repository）→ **struct**（如 `StatsRecorder` 持有 usage+settings repo，`ModelRegistry` 持有 pool）。
- 无需为每实体造 service 类（那是 DDD 过度）。

**不放什么**：HTTP 解析与响应格式、raw SQL（调 repository，不写 SQL）。

### 4.3 repository（数据访问层）

**职责**：业务实体的存取，**所有 SQL 收在此**。把表行（Row）映射成 domain 实体。

**形态**：`trait + impl 同文件`的务实薄分层（galaxy 已采用，7 个实体已就位）。trait 定义契约（可未来换 impl/mock），`Sqlite*Repository` 是当前实现，`Repositories` 容器统一持有。

> **为何 SQL impl 不进 infra（严格 clean 派做法）？** 严格 clean 会把 SQL adapter 放 `infrastructure/persistence/`、trait 放 `domain/`。galaxy 选务实派：trait+impl 同放 `repository/`。理由：单 SQLite（约束 #5 锁定）、无多源切换、无 mock 单测需求，严格 port/adapter 分离收益≈0 而成本（每实体分两处）实打实。见 §9.3 升级条件。

**不放什么**：业务规则（不在这里做"创建渠道时校验 endpoint 合法性"——那是 service）、HTTP、缓存策略编排（缓存机制在 infra，何时刷在 service）。

### 4.4 domain（领域层）

**职责**：纯数据形状（实体 struct）+ 不依赖框架的简单业务规则。允许 `serde`。

**不放什么**：任何运行时框架（`axum/sqlx/reqwest`）、SQL、HTTP。

### 4.5 infra（基础设施层）

**职责**：技术设施——怎么连数据库、怎么缓存、怎么读配置、怎么发 HTTP。**不认识业务实体**。

**判据**：换掉所有业务实体（channel/route/key），这块代码还在不在？在 → infra。

**当前内容**（已就位、符合基准）：`db/`（连接池 + 迁移执行器）、`cache.rs`（ProxyCache）、`config.rs`。

**不放什么**：业务实体的存取逻辑（那是 repository）、业务编排（那是 service）。

> `ProxyCache` 缓存的是 `ChannelInfo`/`RouteInfo`（业务类型），但缓存**机制**本身通用——它是技术设施，缓存什么由调用方（service）决定，故归 infra。

### 4.6 llm（转发引擎层）

**职责**：LLM 请求转发管道的聚合层。`relay`（编排）→ `protocol`（转换）/ `scheduler`（选渠道）/ `plugin`（改写）。与 service 平级，依赖 infra（reqwest）。

**为何独立成层**：转发链路是 galaxy 的核心，调研四个参考 LLM 网关无一把转发做成多个平级顶层包。聚合进 `llm/` 后，relay 调 protocol/scheduler/plugin 从跨顶层变成引擎内部调用，依赖最内聚。

---

## 5. 边界判据（决策指南）

### 5.1 一段代码该放哪——三问

1. **它是某个业务实体（channel/route/key）专属的存取吗？** → `repository/`
2. **它换掉业务实体还成立（纯连接/缓存/配置机制）吗？** → `infra/`
3. **它含业务规则、事务编排、多步副作用吗？** → `service/`
4. 都不是，只是 HTTP 解析/响应？→ `api/`
5. 只是数据形状？→ `domain/`

**铁律**：
- **SQL 只出现在 `repository/`**。`app_state.rs`、`backup.rs`、handler 里写 SQL = 违规。
- handler/service **不直接持有 `pool`、不写 `sqlx::query`**——要数据走 repository。
- handler 不含业务逻辑；service 不碰 HTTP；repository 不含业务规则。

### 5.2 何时可以跳过 service

仅当某 handler 操作是**零业务逻辑的纯透传**（无校验、无跨表、无副作用、无缓存协调）时，可 `handler → repository`。应作为少数例外，且代码中能清晰解释为何不需要 service。galaxy 的常态是 `handler → service → repository`。

### 5.3 何时升级到严格 clean

当前务实派（trait+impl 同放 `repository/`）。出现以下任一情况时升级为严格派（trait 入 `domain/`、SQL impl 入 `infra/persistence/`）：

- 需要支持第二个数据库（如 Postgres）。
- 引入基于 mock repository 的单元测试实践。
- 团队规模扩大，需要严格的层隔离契约。

在此之前，强制 port/adapter 分离是过度设计。

---

## 6. 错误类型分层

按"返回格式"而非"代码层级"划分，两类：

| 错误类型 | 含义 | 返回方式 | 适用 |
|---|---|---|---|
| `ProxyError`（`error::proxy`） | 代理转发链路错误：上游非 2xx、协议转换失败、超时、relay 内部 | **原生透传**，不统一包装 | 代理 API `/v1/*` |
| `ServiceError`（`error::service`，即原 `error::app`） | 服务侧错误：DB 故障、参数校验、业务规则、配置 | **统一封装** `{code,message,data}` | 管理 API `/api/v1/admin/*` |

- repository **不单独定义错误**：sqlx 错误在 service 层映射为 `ServiceError`。
- domain 不定义错误。
- 反模式：在 handler 用 `e.to_string().contains("UNIQUE constraint")` 嗅探 SQL constraint——应转 typed error。

---

## 7. 工程约束（不变量）

| # | 约束 | 原因 |
|---|------|------|
| 1 | 所有实体 ID 用 UUID v7，TEXT 类型 | 全局唯一 + 时序可排序 |
| 2 | 管理 API（`/api/v1/admin/*`）统一 JSON：`{code,message,data}` | 与代理 API 分离 |
| 3 | 代理 API（`/v1/*`）保持原生协议格式，不统一包装 | 透传上游，SDK 兼容 |
| 4 | 渠道 `models` 字段固定为 JSON 字符串数组 | 多处代码依赖此结构 |
| 5 | 迁移 SQL 放 `src/db/migrations/`，文件名 `{version}_{name}.sql`，version>0，不可回滚/改已发布版本 | `sqlx::migrate!()` 编译期管理 |
| 6 | 业务配置变更后必须失效对应 `ProxyCache` | 缓存与数据库一致性 |

---

## 8. 现状对照与债清单（待消除）

galaxy 的 repository 层（trait+impl）已基本就位且形态正确（7 实体 + Repositories 容器），infra 三件套（db/cache/config）也符合基准。**核心债是"上层绕过现成的 repository"** + service 层不完整 + 迁移 shim 残留：

| 优先级 | 债 | 现状 | 目标 |
|---|---|---|---|
| 🔴 | **AppState 写 proxy 热路径 SQL** | `app_state.rs` 的 `find_route_by_name`/`find_route_by_regex`/`get_route_items`/`get_channel` 直连 `self.pool`，绕过现成的 `RouteRepository`/`ChannelRepository` | SQL 下沉 repository，编排（缓存协调）进 `service/route`、`service/channel` |
| 🔴 | **backup.rs raw SQL** | 428 行 export/import/reset 直连 `state.pool` | 事务编排进 `service/backup`，SQL 进 repository |
| 🔴 | **service 层不完整** | 仅 `pricing/`+`stats/`；admin 各业务逻辑散在 handler | 补 `channel/route/api_key/settings/backup` 等 service（见 §3） |
| 🟡 | **迁移 shim 残留** | `lib.rs` 顶层 `pub use crate::llm::{relay,scheduler,protocol}` 让旧路径 `crate::relay` 可用；`app_state` 仍用旧路径 | 撕 shim，调用方切到 `crate::llm::` |
| 🟡 | **Services 空壳** | `service/mod.rs` 的 `Services` struct 是 `#[allow(dead_code)]` 空壳 | 删除（design §2.4 不要 service 容器；真服务按业务能力独立成模块） |
| 🟡 | **SQL constraint 嗅探** | handler 里 `e.to_string().contains("UNIQUE...")` ×4 | 转 typed error |
| 🟢 | **service 形态代码位置** | `update_check.rs` 的 `UpdateCheckContext` 已是 service 形态却留在 handler 文件；`channels/probe.rs` 探测逻辑塞 handler 模块 | 归位到对应 service 模块 |

> 已对齐（无需动）：domain/repository/infra/llm/api 的目录骨架；repository 的 trait+impl 务实形态；infra 的 db/cache/config 三件套。

---

## 9. 演进与落地规则

### 9.1 新增功能时的落点顺序

1. 先定 `domain/` 实体（数据形状）。
2. 加 `repository/` 存取（trait + impl + 挂进 `Repositories`）。
3. 写 `service/` 业务逻辑（调 repository、协调缓存）。
4. 暴露 `api/` handler（调 service、格式化响应）。

### 9.2 大文件拆分原则

> 500 行的文件应拆分（design §3.4）。拆分按"职责/协议/查询类型"切，不按"行数均分"。

### 9.3 升级触发条件

见 §5.3。在触发前，保持务实薄分层，不预先做 port/adapter 分离。

### 9.4 调整批次原则

- 每个批次只做一件可验证的事（如"撕 relay shim"或"下沉 route SQL"），`make check` 全绿。
- shim-first 渐进迁移可用，但**必须配套清理批次**——搬完不留旧路径 shim（当前 lib.rs 的三个 shim 就是"搬完没清"的教训）。
