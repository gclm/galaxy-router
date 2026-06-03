# 开发方案

## 开发策略

- **TDD**: 先写测试，再写实现
- **数据库迁移**: `sqlx migrate`
- **增量开发**: 每个阶段完成后可独立运行和测试

## 阶段划分

### Phase 1: 项目骨架（预计 2 天）

**目标**: 搭建基础框架，能编译运行

| 任务 | 说明 | 依赖 |
|------|------|------|
| 1.1 初始化 Cargo 项目 | workspace 结构，配置依赖 | 无 |
| 1.2 TOML 配置加载 | 解析 config.toml | 1.1 |
| 1.3 数据库初始化 | SQLite 连接 + Schema 迁移 | 1.1 |
| 1.4 日志系统 | tracing 初始化 | 1.1 |
| 1.5 HTTP 服务骨架 | axum 启动 + 健康检查端点 | 1.1, 1.2 |

**验证**: `cargo run` 启动服务，访问 `/health` 返回 200

```bash
# 测试命令
cargo test
curl http://localhost:8080/health
```

---

### Phase 2: 认证系统（预计 1 天）

**目标**: 管理员初始化和登录

| 任务 | 说明 | 依赖 |
|------|------|------|
| 2.1 users 表迁移 | 创建 users 表 | 1.3 |
| 2.2 密码哈希 | argon2id 加密/验证 | 1.1 |
| 2.3 JWT 工具 | 生成/验证 Token | 1.2 |
| 2.4 初始化 API | `POST /api/v1/admin/auth/setup` | 2.1, 2.2, 2.3 |
| 2.5 登录 API | `POST /api/v1/admin/auth/login` | 2.1, 2.2, 2.3 |
| 2.6 认证中间件 | JWT 验证中间件 | 2.3 |
| 2.7 修改密码 API | `PUT /api/v1/admin/auth/password` | 2.6 |

**测试**:
```rust
#[tokio::test]
async fn test_setup_admin() {
    // 首次初始化成功
    // 重复初始化返回 409
}

#[tokio::test]
async fn test_login() {
    // 正确密码返回 token
    // 错误密码返回 401
}
```

**验证**: 调用 setup → login → 获取 token → 访问受保护端点

---

### Phase 3: 渠道管理（预计 2 天）

**目标**: CRUD 渠道配置

| 任务 | 说明 | 依赖 |
|------|------|------|
| 3.1 channels 表迁移 | 创建 channels 表 | 1.3 |
| 3.2 渠道 Model | Rust 结构体 + serde | 3.1 |
| 3.3 渠道 Repository | CRUD 数据库操作 | 3.2 |
| 3.4 渠道 API | RESTful 端点 | 2.6, 3.3 |
| 3.5 渠道缓存 | 内存 HashMap + RwLock | 3.3 |

**API 端点**:
```
GET    /api/v1/admin/channels          # 列表
POST   /api/v1/admin/channels          # 创建
GET    /api/v1/admin/channels/:id      # 详情
PUT    /api/v1/admin/channels/:id      # 更新
DELETE /api/v1/admin/channels/:id      # 删除
```

**测试**:
```rust
#[tokio::test]
async fn test_create_channel() {
    // 创建成功返回 201
    // 重复名称返回 409
    // 无效类型返回 400
}

#[tokio::test]
async fn test_channel_cache() {
    // 创建后缓存立即更新
    // 删除后缓存立即清除
}
```

---

### Phase 4: 分组管理（预计 2 天）

**目标**: CRUD 分组配置

| 任务 | 说明 | 依赖 |
|------|------|------|
| 4.1 groups 表迁移 | 创建 groups 表 | 1.3 |
| 4.2 group_items 表迁移 | 创建 group_items 表 | 1.3 |
| 4.3 分组 Model | Rust 结构体 + serde | 4.1, 4.2 |
| 4.4 分组 Repository | CRUD 数据库操作 | 4.3 |
| 4.5 分组 API | RESTful 端点 | 2.6, 4.4 |
| 4.6 分组缓存 | 内存 HashMap + RwLock | 4.4 |

**API 端点**:
```
GET    /api/v1/admin/groups            # 列表
POST   /api/v1/admin/groups            # 创建
GET    /api/v1/admin/groups/:id        # 详情
PUT    /api/v1/admin/groups/:id        # 更新
DELETE /api/v1/admin/groups/:id        # 删除
POST   /api/v1/admin/groups/:id/items  # 添加分组项
DELETE /api/v1/admin/groups/:id/items/:item_id  # 删除分组项
```

---

### Phase 5: 客户端 API Key（预计 1 天）

**目标**: 管理客户端访问 Proxy 的 Key

| 任务 | 说明 | 依赖 |
|------|------|------|
| 5.1 api_keys 表迁移 | 创建 api_keys 表 | 1.3 |
| 5.2 API Key Model | Rust 结构体 | 5.1 |
| 5.3 API Key Repository | CRUD 操作 | 5.2 |
| 5.4 API Key API | RESTful 端点 | 2.6, 5.3 |
| 5.5 API Key 认证中间件 | 验证客户端 Key | 5.3 |

**API 端点**:
```
GET    /api/v1/admin/api-keys          # 列表
POST   /api/v1/admin/api-keys          # 创建
DELETE /api/v1/admin/api-keys/:id      # 删除
PUT    /api/v1/admin/api-keys/:id      # 启用/禁用
```

---

### Phase 6: 协议转换核心（预计 5 天）

**目标**: 实现三种协议的双向转换

| 任务 | 说明 | 依赖 |
|------|------|------|
| 6.1 统一内部模型 | LlmRequest, LlmResponse | 1.1 |
| 6.2 Inbound trait | 定义入站转换接口 | 6.1 |
| 6.3 Outbound trait | 定义出站转换接口 | 6.1 |
| 6.4 OpenAI Chat Inbound | 客户端 → 统一格式 | 6.2 |
| 6.5 OpenAI Chat Outbound | 统一格式 → 上游 | 6.3 |
| 6.6 OpenAI Responses Inbound | 客户端 → 统一格式 | 6.2 |
| 6.7 OpenAI Responses Outbound | 统一格式 → 上游 | 6.3 |
| 6.8 Anthropic Inbound | 客户端 → 统一格式 | 6.2 |
| 6.9 Anthropic Outbound | 统一格式 → 上游 | 6.3 |
| 6.10 SSE 解析器 | 流式事件解析 | 1.1 |
| 6.11 流式转换 | Stream<Event> ↔ Stream<Response> | 6.10 |

**测试策略** (Round-trip 测试):
```rust
#[tokio::test]
async fn test_openai_to_anthropic_roundtrip() {
    // OpenAI 请求 → 统一格式 → Anthropic 请求 → Anthropic 响应 → 统一格式 → OpenAI 响应
    // 验证关键字段不丢失
}

#[tokio::test]
async fn test_streaming_conversion() {
    // 流式事件逐个转换
    // 验证终止事件正确识别
}
```

**参考**: AxonHub `llm/transformer/` 目录

---

### Phase 7: 代理转发（预计 3 天）

**目标**: 请求分发和转发

| 任务 | 说明 | 依赖 |
|------|------|------|
| 7.1 协议识别 | 根据端点/UA 识别协议 | 6.4-6.9 |
| 7.2 分组匹配 | model → group 匹配逻辑 | 4.6 |
| 7.3 渠道选择 | 从 group 选择 channel | 3.5 |
| 7.4 模型映射 | 应用 channel.model_maps | 7.3 |
| 7.5 HTTP 转发器 | 非流式请求转发 | 7.4 |
| 7.6 流式转发器 | SSE 流式转发 | 7.4, 6.10 |
| 7.7 同格式直通 | 跳过转换直接转发 | 7.4 |

**API 端点**:
```
POST /v1/chat/completions    # OpenAI Chat
POST /v1/responses           # OpenAI Responses
POST /v1/messages            # Anthropic Messages
GET  /v1/models              # 模型列表
```

**测试**:
```rust
#[tokio::test]
async fn test_proxy_openai_to_anthropic() {
    // 模拟 OpenAI 请求
    // 验证转发到 Anthropic 格式
    // 验证响应正确转换
}

#[tokio::test]
async fn test_passthrough_same_format() {
    // 同格式请求走直通路径
    // 验证无转换开销
}
```

---

### Phase 8: 负载均衡（预计 2 天）

**目标**: 加权评分选择 + 粘性会话

| 任务 | 说明 | 依赖 |
|------|------|------|
| 8.1 评分模型 | 实现加权评分公式 | 7.3 |
| 8.2 Top-K 选择 | 取 Top-K 候选 | 8.1 |
| 8.3 加权随机 | 轮盘赌选择 | 8.2 |
| 8.4 粘性会话 | 内存 HashMap + TTL | 8.3 |
| 8.5 故障转移 | 失败重试 + 渠道切换 | 8.4 |

**测试**:
```rust
#[tokio::test]
async fn test_weighted_selection() {
    // 验证权重分布符合预期
}

#[tokio::test]
async fn test_sticky_session() {
    // 同一 session_hash 路由到同一渠道
}

#[tokio::test]
async fn test_failover() {
    // 主渠道失败自动切换到备渠道
}
```

---

### Phase 9: 统计系统（预计 2 天）

**目标**: 记录用量和成本

| 任务 | 说明 | 依赖 |
|------|------|------|
| 9.1 usage_logs 表迁移 | 创建用量日志表 | 1.3 |
| 9.2 usage_daily 表迁移 | 创建按天聚合表 | 1.3 |
| 9.3 统计记录器 | 每次请求写入 usage_logs | 9.1 |
| 9.4 按天聚合 | 定时任务聚合到 usage_daily | 9.2, 9.3 |
| 9.5 成本计算 | 从 models.dev 拉取定价 | 1.2 |
| 9.6 定价覆盖 | 本地 model_pricing 表 | 9.5 |
| 9.7 统计 API | 查询接口 | 2.6, 9.3 |

**API 端点**:
```
GET /api/v1/admin/stats/overview     # 总览
GET /api/v1/admin/stats/daily        # 按天统计
GET /api/v1/admin/stats/models       # 按模型统计
GET /api/v1/admin/stats/channels     # 按渠道统计
```

---

### Phase 10: 前端管理面板（预计 5 天）

**目标**: React Web 管理界面

| 任务 | 说明 | 依赖 |
|------|------|------|
| 10.1 前端项目初始化 | Vite + React + TypeScript | 无 |
| 10.2 UI 组件库 | shadcn/ui 集成 | 10.1 |
| 10.3 API 客户端 | fetch 封装 + 类型定义 | 10.1 |
| 10.4 登录页面 | 表单 + Token 存储 | Phase 2 |
| 10.5 渠道管理页面 | 列表 + 表单 | Phase 3 |
| 10.6 分组管理页面 | 列表 + 表单 | Phase 4 |
| 10.7 API Key 页面 | 列表 + 生成 | Phase 5 |
| 10.8 统计仪表盘 | 图表 + 数据展示 | Phase 9 |
| 10.9 嵌入 Rust | rust-embed 打包 | 10.1 |

---

### Phase 11: 端点扩展（预计 1 天）

**目标**: Embedding 和 Images API

| 任务 | 说明 | 依赖 |
|------|------|------|
| 11.1 Embedding API | `/v1/embeddings` 直通 | Phase 7 |
| 11.2 Images API | `/v1/images/generations` 直通 | Phase 7 |
| 11.3 模型列表 API | `/v1/models` | Phase 7 |

---

### Phase 12: 部署与打包（预计 1 天）

**目标**: 单二进制 + Docker

| 任务 | 说明 | 依赖 |
|------|------|------|
| 12.1 Release 构建 | 优化编译配置 | 全部 |
| 12.2 Dockerfile | 多阶段构建 | 12.1 |
| 12.3 CLI 参数 | clap 参数解析 | 1.1 |
| 12.4 README | 使用文档 | 全部 |

---

## 开发顺序总览

```
Phase 1: 项目骨架 (2天)
    │
    ├─→ Phase 2: 认证系统 (1天)
    │       │
    │       ├─→ Phase 3: 渠道管理 (2天)
    │       │       │
    │       │       └─→ Phase 8: 负载均衡 (2天)
    │       │
    │       ├─→ Phase 4: 分组管理 (2天)
    │       │
    │       └─→ Phase 5: API Key (1天)
    │
    ├─→ Phase 6: 协议转换 (5天) ← 核心模块
    │       │
    │       └─→ Phase 7: 代理转发 (3天)
    │               │
    │               ├─→ Phase 11: 端点扩展 (1天)
    │               │
    │               └─→ Phase 9: 统计系统 (2天)
    │
    └─→ Phase 10: 前端面板 (5天)
            │
            └─→ Phase 12: 部署打包 (1天)
```

## 总预估时间

| 阶段 | 时间 |
|------|------|
| Phase 1-5 (基础) | 8 天 |
| Phase 6-7 (核心) | 8 天 |
| Phase 8-9 (增强) | 4 天 |
| Phase 10 (前端) | 5 天 |
| Phase 11-12 (收尾) | 2 天 |
| **总计** | **27 天** |

## 关键风险

| 风险 | 影响 | 缓解措施 |
|------|------|---------|
| 协议转换复杂度 | Phase 6 延期 | 参考 AxonHub 实现，先做 Chat Completions |
| 流式处理边界 | Phase 7 延期 | 先做非流式，再加流式支持 |
| 前端工作量 | Phase 10 延期 | 初版只做核心功能，样式简化 |
