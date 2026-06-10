# Galaxy Router 回归测试方案

> 版本: v1.0 | 日期: 2026-06-10

## 1. 目标

代码改动后，**一条命令**完成所有 API 接口的正常 + 异常路径回归验证，覆盖：
- 管理后台全部 CRUD 操作
- 代理转发认证与路由
- 协议转换正确性
- 关键业务场景（备份恢复、缓存失效、故障转移）

## 2. 现状与差距

### 现有测试（85 个）

| 文件 | 数量 | 层级 |
|------|------|------|
| `tests/integration_test.rs` | ~15 | 单元（DB/密码/JWT/序列化） |
| `tests/protocol_integration.rs` | ~35 | 单元（协议转换） |
| `tests/e2e_integration.rs` | ~20 | HTTP（oneshot，认证/路由可达/部分 CRUD） |
| `tests/sse_integration.rs` | ~15 | 单元（SSE 解析） |

### 主要问题

1. **重复构建** — `build_test_app()` / `build_test_app_with_key()` / `build_test_app_with_admin()` 各自建 DB + Router，代码重复 ~200 行
2. **覆盖缺口** — 渠道完整 CRUD、分组完整 CRUD、备份导入/重置、预算管理、统计接口补全均未覆盖
3. **无 Mock** — 代理测试只能验证到"模型不存在 404"，无法验证请求转发/协议转换/流式响应的完整链路
4. **无分类执行** — `make test` 全量跑，无法按模块选择性执行

## 3. 目录结构

```
tests/
├── common/                          # 共享测试基础设施
│   ├── mod.rs
│   ├── app.rs                       # TestApp 构建（DB + Router + JWT + API Key）
│   └── mock.rs                      # wiremock 上游模拟
│
├── api/                             # 管理后台 HTTP 回归（按资源分文件）
│   ├── auth.rs                      # 登录 /me /password /init
│   ├── channels.rs                  # 渠道 CRUD + test + detect
│   ├── groups.rs                    # 分组 CRUD + items
│   ├── api_keys.rs                  # API Key CRUD
│   ├── stats.rs                     # 统计 6 端点
│   ├── settings.rs                  # 设置 list/infra/update
│   ├── backup.rs                    # export/import/reset
│   ├── models_info.rs               # 模型信息 list/get/update
│   └── system.rs                    # 系统信息 / 健康检查
│
├── proxy/                           # 代理转发全链路（需要 mock 上游）
│   ├── chat.rs                      # /v1/chat/completions
│   ├── messages.rs                  # /v1/messages (Anthropic)
│   ├── responses.rs                 # /v1/responses
│   ├── embeddings.rs                # /v1/embeddings
│   ├── images.rs                    # /v1/images/generations
│   ├── models_list.rs               # GET /v1/models
│   └── failover.rs                  # 渠道故障转移
│
├── integration_test.rs              # 保留：DB/密码/JWT 单元测试
├── protocol_integration.rs          # 保留：协议转换单元测试
└── sse_integration.rs               # 保留：SSE 解析单元测试
```

## 4. 基础设施

### 4.1 TestApp（`tests/common/app.rs`）

统一测试应用构建，消除重复代码：

```rust
pub struct TestApp {
    pub router: Router,
    pub db_path: String,
    pub jwt_secret: String,
    pub admin_jwt: String,
    pub admin_user_id: String,
}

impl TestApp {
    /// 最小构建：空 DB + Router，无用户
    pub async fn new_empty() -> Self { ... }

    /// 标准构建：含 admin 用户（username="admin", password="password123"）
    pub async fn new() -> Self { ... }

    /// 完整构建：admin + API Key + 示例渠道 + 分组
    pub async fn new_with_fixtures() -> Self { ... }

    // ── 请求构造 ──

    /// 发送带 admin JWT 的请求
    pub fn admin_req(&self, method: Method, uri: &str) -> Request<Body> { ... }

    /// 发送带 admin JWT + JSON body 的请求
    pub fn admin_json(&self, method: Method, uri: &str, body: &str) -> Request<Body> { ... }

    /// 发送带 API Key 的代理请求
    pub fn proxy_req(&self, method: Method, uri: &str, body: &str, api_key: &str) -> Request<Body> { ... }

    /// 发送无认证的请求
    pub fn anon_req(&self, method: Method, uri: &str) -> Request<Body> { ... }

    // ── 辅助方法 ──

    /// 通过 SQL 直接插入 API Key，返回 (id, key_string)
    pub async fn insert_api_key(&self, name: &str, enabled: bool) -> (String, String) { ... }

    /// 通过 SQL 直接插入渠道，返回 channel_id
    pub async fn insert_channel(&self, name: &str, base_url: &str) -> String { ... }

    /// 通过 SQL 直接插入分组，返回 group_id
    pub async fn insert_group(&self, name: &str, channel_id: &str, model: &str) -> String { ... }
}
```

关键设计决策：
- **每个 `TestApp::new()` 创建独立 SQLite 文件**（UUID v7 命名），测试间无数据污染
- **Router 内部共享连接池**，`Router::clone()` 不会重建
- **`admin_jwt` 在构建时生成**，所有测试用同一个有效 JWT，无需每次重新登录
- **提供 SQL 级辅助方法** 用于快速造测试数据（绕过 API 验证）

### 4.2 Mock Server（`tests/common/mock.rs`）

使用 `wiremock` 模拟上游 LLM Provider：

```rust
pub async fn spawn_openai_chat_mock(body_response: serde_json::Value) -> MockServer { ... }
pub async fn spawn_anthropic_messages_mock(body_response: serde_json::Value) -> MockServer { ... }
pub async fn spawn_openai_embeddings_mock() -> MockServer { ... }
pub async fn spawn_openai_images_mock() -> MockServer { ... }
pub async fn spawn_openai_responses_mock() -> MockServer { ... }
pub async fn spawn_timeout_mock(delay: Duration) -> MockServer { ... }
pub async fn spawn_error_mock(status: u16) -> MockServer { ... }
```

### 4.3 依赖

```toml
# Cargo.toml [dev-dependencies] 新增
wiremock = "0.6"
```

## 5. 测试用例矩阵

每个端点覆盖 **正常路径**（Happy Path）和 **异常路径**（Error Path）。

### 5.1 Auth（`tests/api/auth.rs`）

| # | 用例 | 路径 | 验证 |
|---|------|------|------|
| 1 | POST /api/v1/init — 首次初始化 | 正常 | 201，返回 JWT |
| 2 | POST /api/v1/init — 重复初始化 | 异常 | 409 conflict |
| 3 | POST /api/v1/init — 用户名太短 | 异常 | 400 |
| 4 | POST /api/v1/init — 密码太短 | 异常 | 400 |
| 5 | POST /api/v1/admin/auth/login — 正确密码 | 正常 | 200，返回 JWT |
| 6 | POST /api/v1/admin/auth/login — 错误密码 | 异常 | 401 |
| 7 | POST /api/v1/admin/auth/login — 不存在的用户 | 异常 | 401 |
| 8 | GET /api/v1/admin/auth/me — 有效 JWT | 正常 | 200，返回用户信息 |
| 9 | GET /api/v1/admin/auth/me — 无 JWT | 异常 | 401 |
| 10 | GET /api/v1/admin/auth/me — 无效 JWT | 异常 | 401 |
| 11 | PUT /api/v1/admin/auth/password — 正确旧密码 | 正常 | 200 |
| 12 | PUT /api/v1/admin/auth/password — 错误旧密码 | 异常 | 401 |
| 13 | PUT /api/v1/admin/auth/password — 新密码太短 | 异常 | 400 |

### 5.2 Channels（`tests/api/channels.rs`）

| # | 用例 | 路径 | 验证 |
|---|------|------|------|
| 1 | POST create — 完整字段 | 正常 | 201，验证返回结构 |
| 2 | POST create — 最小必填字段 | 正常 | 201 |
| 3 | POST create — 空名称 | 异常 | 400 |
| 4 | POST create — 空 api_keys | 异常 | 400 |
| 5 | POST create — 空 endpoints | 异常 | 400 |
| 6 | POST create — API Key 含 CRLF | 异常 | 400 |
| 7 | GET list — 空列表 | 正常 | 200，items=[] |
| 8 | GET list — 有数据 + 分页 + 搜索 | 正常 | 200，验证 total/page |
| 9 | GET list — 按状态筛选 | 正常 | 200 |
| 10 | GET by id — 存在 | 正常 | 200 |
| 11 | GET by id — 不存在 | 异常 | 404 |
| 12 | PUT update — 修改名称 | 正常 | 200，验证名称已变更 |
| 13 | PUT update — 修改 models | 正常 | 200 |
| 14 | PUT update — 禁用渠道 | 正常 | 200，enabled=false |
| 15 | PUT update — 不存在的渠道 | 异常 | 404 |
| 16 | PUT update — 无字段更新 | 异常 | 400 |
| 17 | DELETE — 存在的渠道 | 正常 | 200 |
| 18 | DELETE — 不存在的渠道 | 异常 | 404 |
| 19 | POST test — 正常连通性测试 | 正常 | 200，返回 latency（需 mock） |
| 20 | POST detect — 正常检测 | 正常 | 200（需 mock） |

### 5.3 Groups（`tests/api/groups.rs`）

| # | 用例 | 路径 | 验证 |
|---|------|------|------|
| 1 | POST create — 含 items | 正常 | 201 |
| 2 | POST create — 空名称 | 异常 | 400 |
| 3 | POST create — 空 items | 异常 | 400 |
| 4 | POST create — 不存在的 channel_id | 异常 | 400 |
| 5 | GET list — 空列表 | 正常 | 200 |
| 6 | GET list — 有数据 + 分页 | 正常 | 200 |
| 7 | GET by id — 存在 | 正常 | 200，含 items |
| 8 | GET by id — 不存在 | 异常 | 404 |
| 9 | PUT update — 修改名称和 items | 正常 | 200 |
| 10 | PUT update — 空 items | 异常 | 400 |
| 11 | PUT update — 不存在 | 异常 | 404 |
| 12 | DELETE — 存在 | 正常 | 200 |
| 13 | DELETE — 不存在 | 异常 | 404 |
| 14 | POST add_item — 正常添加 | 正常 | 201 |
| 15 | POST add_item — 不存在的 group | 异常 | 404 |
| 16 | DELETE item — 正常删除 | 正常 | 200 |
| 17 | DELETE item — 不存在 | 异常 | 404 |

### 5.4 API Keys（`tests/api/api_keys.rs`）

| # | 用例 | 路径 | 验证 |
|---|------|------|------|
| 1 | POST create — 正常 | 正常 | 201，验证 key 以 "gp-" 开头 |
| 2 | POST create — 空名称 | 异常 | 400 |
| 3 | GET list — 空列表 | 正常 | 200 |
| 4 | GET list — 有数据 | 正常 | 200 |
| 5 | GET by id — 存在 | 正常 | 200 |
| 6 | GET by id — 不存在 | 异常 | 404 |
| 7 | PUT update — 修改名称 | 正常 | 200 |
| 8 | PUT update — 禁用 | 正常 | 200，enabled=false |
| 9 | PUT update — 无字段 | 异常 | 400 |
| 10 | PUT update — 不存在 | 异常 | 404 |
| 11 | DELETE — 存在 | 正常 | 200 |
| 12 | DELETE — 不存在 | 异常 | 404 |
| 13 | PUT update 禁用后 → 代理请求 | 异常 | 403 |

### 5.5 Stats（`tests/api/stats.rs`）

| # | 用例 | 路径 | 验证 |
|---|------|------|------|
| 1 | GET overview — 空数据 | 正常 | 200，code=0 |
| 2 | GET models — 默认天数 | 正常 | 200 |
| 3 | GET models — 自定义日期范围 | 正常 | 200 |
| 4 | GET channels — 正常 | 正常 | 200 |
| 5 | GET daily — 正常 | 正常 | 200 |
| 6 | GET api-keys — 正常 | 正常 | 200 |
| 7 | GET latency — 正常 | 正常 | 200 |
| 8 | GET logs — 空日志 | 正常 | 200 |
| 9 | POST budgets — 设置预算 | 正常 | 200 |
| 10 | GET budgets — 列表 | 正常 | 200 |
| 11 | DELETE budgets/{id} — 删除 | 正常 | 200 |
| 12 | GET logs/{id} — 不存在 | 异常 | 404 |

### 5.6 Settings（`tests/api/settings.rs`）

| # | 用例 | 路径 | 验证 |
|---|------|------|------|
| 1 | GET list — 正常 | 正常 | 200 |
| 2 | GET infra — 正常 | 正常 | 200，含 server/database/logging/auth |
| 3 | PUT update — 白名单内 key | 正常 | 200 |
| 4 | PUT update — 白名单外 key | 异常 | 400 |
| 5 | PUT update — 不存在的 key | 异常 | 404 |

### 5.7 Backup（`tests/api/backup.rs`）

| # | 用例 | 路径 | 验证 |
|---|------|------|------|
| 1 | GET export — 空数据 | 正常 | 200，format/version/data 结构正确 |
| 2 | GET export — 有数据 | 正常 | 200，包含 channels/groups/api_keys/settings |
| 3 | POST import — 正常导入 | 正常 | 200，计数正确 |
| 4 | POST import — 格式错误 | 异常 | 400 |
| 5 | POST import — 版本不匹配 | 异常 | 400 |
| 6 | POST reset — 正常重置 | 正常 | 200，计数正确 |
| 7 | Export → Reset → Import 往返 | 正常 | 数据恢复一致 |

### 5.8 Models Info（`tests/api/models_info.rs`）

| # | 用例 | 路径 | 验证 |
|---|------|------|------|
| 1 | GET list — 空列表 | 正常 | 200 |
| 2 | GET by model — 不存在 | 异常 | 404 |
| 3 | PUT update — 批量更新 | 正常 | 200 |

### 5.9 System（`tests/api/system.rs`）

| # | 用例 | 路径 | 验证 |
|---|------|------|------|
| 1 | GET /api/v1/health — 未初始化 | 正常 | 200，needs_setup=true |
| 2 | GET /api/v1/health — 已初始化 | 正常 | 200，needs_setup=false |
| 3 | GET /api/v1/admin/system-info — 正常 | 正常 | 200 |

### 5.10 Proxy（`tests/proxy/*.rs`）

代理测试需要 mock 上游。先用 `TestApp::new_with_fixtures()` 创建含渠道的应用，
但渠道的 `base_url` 指向 wiremock 启动的本地地址。

#### Chat Completions（`tests/proxy/chat.rs`）

| # | 用例 | 路径 | 验证 |
|---|------|------|------|
| 1 | 正常非流式请求 | 正常 | 200，响应结构正确 |
| 2 | 正常流式请求 | 正常 | 200，SSE 事件流 |
| 3 | 无 API Key | 异常 | 401 |
| 4 | 禁用的 API Key | 异常 | 403 |
| 5 | 无效 API Key | 异常 | 401 |
| 6 | 模型不存在 | 异常 | 404 |
| 7 | 上游返回错误 | 异常 | 透传上游状态码 |

#### Messages / Anthropic（`tests/proxy/messages.rs`）

| # | 用例 | 路径 | 验证 |
|---|------|------|------|
| 1 | 正常非流式请求 | 正常 | 200 |
| 2 | x-api-key 认证 | 正常 | 200 |
| 3 | anthropic-version 头处理 | 正常 | 200 |
| 4 | 无认证 | 异常 | 401 |

#### Responses（`tests/proxy/responses.rs`）

| # | 用例 | 路径 | 验证 |
|---|------|------|------|
| 1 | 正常请求 | 正常 | 200 |
| 2 | 无 API Key | 异常 | 401 |

#### Embeddings（`tests/proxy/embeddings.rs`）

| # | 用例 | 路径 | 验证 |
|---|------|------|------|
| 1 | 正常请求 | 正常 | 200 |
| 2 | 无 API Key | 异常 | 401 |

#### Images（`tests/proxy/images.rs`）

| # | 用例 | 路径 | 验证 |
|---|------|------|------|
| 1 | 正常请求 | 正常 | 200 |
| 2 | 无 API Key | 异常 | 401 |

#### Models List（`tests/proxy/models_list.rs`）

| # | 用例 | 路径 | 验证 |
|---|------|------|------|
| 1 | 正常获取 | 正常 | 200，返回分组列表 |
| 2 | 无 API Key | 异常 | 401 |

#### Failover（`tests/proxy/failover.rs`）

| # | 用例 | 路径 | 验证 |
|---|------|------|------|
| 1 | 主渠道超时 → 备渠道接管 | 正常 | 200 |
| 2 | 所有渠道失败 | 异常 | 502/503 |

## 6. Makefile 目标

```makefile
# 快速单元测试（不含 HTTP 层）
test-unit:
	cargo test --lib

# 管理后台 API 回归
test-api:
	cargo test --test api

# 代理转发回归
test-proxy:
	cargo test --test proxy

# 完整回归（跳过 lint）
test-regression: test-unit
	cargo test --test api --test proxy

# CI 完整流程
test-ci: clippy test-regression
```

## 7. 落地计划

### Step 1：基础设施（预计 1-2h）

**目标**：`tests/common/` 可用，现有 e2e 测试可迁移

1. 创建 `tests/common/mod.rs` + `tests/common/app.rs`
2. 实现 `TestApp::new_empty()` / `TestApp::new()` / `TestApp::new_with_fixtures()`
3. 实现请求构造辅助方法（`admin_req` / `admin_json` / `proxy_req` / `anon_req`）
4. 实现 SQL 辅助方法（`insert_api_key` / `insert_channel` / `insert_group`）
5. 验证：用 `TestApp` 重写 `tests/e2e_integration.rs` 的 `build_test_app_with_admin`，确认测试全部通过

### Step 2：P0 回归 — 管理 API（预计 2-3h）

**目标**：渠道 + 分组 + API Key 完整 CRUD 覆盖

1. 创建 `tests/api/auth.rs` — 13 个用例
2. 创建 `tests/api/channels.rs` — 20 个用例
3. 创建 `tests/api/groups.rs` — 17 个用例
4. 创建 `tests/api/api_keys.rs` — 13 个用例
5. 创建 `tests/api/settings.rs` — 5 个用例
6. 创建 `tests/api/backup.rs` — 7 个用例
7. 创建 `tests/api/system.rs` — 3 个用例
8. 创建 `tests/api/models_info.rs` — 3 个用例
9. 创建 `tests/api/stats.rs` — 12 个用例
10. 添加 `wiremock` 依赖
11. 创建 `tests/common/mock.rs`
12. 验证：`cargo test --test api` 全部通过

### Step 3：P1 回归 — 代理全链路（预计 2-3h）

**目标**：代理转发完整链路覆盖

1. 创建 `tests/proxy/chat.rs` — 7 个用例
2. 创建 `tests/proxy/messages.rs` — 4 个用例
3. 创建 `tests/proxy/responses.rs` — 2 个用例
4. 创建 `tests/proxy/embeddings.rs` — 2 个用例
5. 创建 `tests/proxy/images.rs` — 2 个用例
6. 创建 `tests/proxy/models_list.rs` — 2 个用例
7. 创建 `tests/proxy/failover.rs` — 2 个用例
8. 验证：`cargo test --test proxy` 全部通过

### 验收标准

| 验收项 | 标准 |
|--------|------|
| 全量测试通过 | `cargo test` 零失败 |
| API 回归覆盖 | 管理 API 全部端点有正常+异常测试 |
| 代理链路覆盖 | 5 个协议端点有 mock 全链路测试 |
| 执行时间 | `cargo test --test api` < 30s（纯内存 oneshot，无网络） |
| 无重复代码 | 测试构建逻辑集中在 `tests/common/` |
| 分类执行 | `make test-api` / `make test-proxy` / `make test-regression` 可独立运行 |

## 8. 测试规范

### 8.1 命名

```
test_{模块}_{操作}_{场景}
```

示例：
- `test_channels_create_happy_path`
- `test_channels_create_empty_name_returns_400`
- `test_proxy_chat_no_api_key_returns_401`

### 8.2 结构

每个测试遵循 Arrange → Act → Assert 三段式：

```rust
#[tokio::test]
async fn test_channels_create_happy_path() {
    // Arrange
    let app = TestApp::new().await;

    // Act
    let resp = app.oneshot(
        app.admin_json(Method::POST, "/api/v1/admin/channels", r#"{ ... }"#)
    ).await.unwrap();

    // Assert
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = to_json(resp).await;
    assert_eq!(body["code"], 0);
    assert!(body["data"]["id"].is_string());
}
```

### 8.3 响应解析辅助

在 `tests/common/mod.rs` 中提供：

```rust
/// 从 axum Response 提取 JSON body
pub async fn to_json(resp: axum::http::Response<Body>) -> serde_json::Value { ... }

/// 断言响应状态码并返回 JSON
pub async fn assert_status(resp: axum::http::Response<Body>, expected: StatusCode) -> serde_json::Value {
    let status = resp.status();
    let body = to_json(resp).await;
    assert_eq!(status, expected, "body: {body}");
    body
}
```

### 8.4 数据隔离

- 每个测试用例调用 `TestApp::new()` 创建独立 SQLite 文件
- 不依赖测试执行顺序
- 不共享可变状态
- 测试结束可选清理（`/tmp/galaxy_test_*`），CI 中 `/tmp` 随容器销毁自动清理

## 9. 预期用例统计

| 模块 | 正常路径 | 异常路径 | 合计 |
|------|----------|----------|------|
| Auth | 4 | 9 | 13 |
| Channels | 12 | 8 | 20 |
| Groups | 10 | 7 | 17 |
| API Keys | 9 | 4 | 13 |
| Stats | 11 | 1 | 12 |
| Settings | 3 | 2 | 5 |
| Backup | 4 | 2 | 6 |
| Models Info | 2 | 1 | 3 |
| System | 3 | 0 | 3 |
| Proxy Chat | 2 | 5 | 7 |
| Proxy Messages | 3 | 1 | 4 |
| Proxy Responses | 1 | 1 | 2 |
| Proxy Embeddings | 1 | 1 | 2 |
| Proxy Images | 1 | 1 | 2 |
| Proxy Models List | 1 | 1 | 2 |
| Proxy Failover | 1 | 1 | 2 |
| **合计** | **69** | **46** | **115** |

加上现有 85 个单元/协议测试，总量约 **200 个**。
