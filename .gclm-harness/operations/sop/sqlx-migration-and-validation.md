# Roadmap: 迁移到 sqlx-cli + 输入校验中间件

## 概述

将手写数据库迁移系统替换为 `sqlx::migrate!()` 宏，合并所有历史迁移为单一初始 schema。新增 Content-Type 校验中间件。

**目标**：
- 删除 ~120 行手写迁移逻辑（`Migration` 结构体、`get_migrations()`、`run_migrations()` 旧实现 + `include_str!("schema.sql")`），改用 `sqlx::migrate!()` 一行调用
- 合并 8 个历史迁移为一个初始 schema，丢弃旧数据
- 获得 `_sqlx_migrations` 表（含 `success` 状态列、校验和验证）
- 新增 `require_json` 中间件，防止非 JSON 请求体到达反序列化层

**前提**：
- 数据可丢弃，所有现有数据库重建
- `sqlx` 的 `migrate` feature 已在 `Cargo.toml` 中启用（已确认）
- `src/db/migrations/` 目录在编译时存在且包含 `.sql` 文件（`sqlx::migrate!` 是编译期宏，编译时扫描目录）

---

## Step 1: 创建迁移文件

**文件**：`src/db/migrations/1_initial_schema.sql`

> sqlx 命名规则为 `{version}_{description}.sql`（数字版本号 + 单下划线），**不是** Flyway 风格的 `V0__xxx`。`sqlx::migrate!()` 宏在编译期按此规则扫描目录，文件名不匹配会被静默忽略。**注意**：version 必须大于 0（`i64` 且 > 0），version 为 0 会被静默忽略。

**合并内容**（与原 schema.sql 的差异）：
- `api_keys` 新增 `supported_models TEXT NOT NULL DEFAULT ''`（原 migration 6）
- `usage_logs` 新增 `ttft_ms INTEGER`、`attempts TEXT`（原 migration 3）
- `usage_logs` 新增 `upstream_key_hint TEXT`（原 migration 4）
- `usage_logs` 新增 `user_agent TEXT`（原 migration 7）
- 新增 5 个 `usage_logs` 索引（原 migration 5）
- 删除 `_migrations` 表定义（由 sqlx 自动管理 `_sqlx_migrations`）
- 删除 migration 1（清理 settings）和 migration 2（DROP model_pricing），因为是全新 schema 无需清理

---

## Step 2: 重写 `db/mod.rs` 迁移逻辑

**文件**：`src/db/mod.rs`

删除（共 ~60 行）：
- `run_migrations()` 旧实现（第 58-98 行）— 含手写 `_migrations` 表创建、逐个执行逻辑
- `Migration` 结构体（第 209-213 行）
- `get_migrations()` 函数（第 216-268 行）— 含 `include_str!("schema.sql")` 引用
- 删除后 `schema.sql` 不再有任何引用，可在 Step 3 安全删除

替换为：
```rust
async fn run_migrations(&self) -> Result<()> {
    sqlx::migrate!("src/db/migrations")
        .run(&self.pool)
        .await
        .map_err(|e| anyhow::anyhow!("数据库迁移失败: {e}"))?;
    Ok(())
}
```

注意：
- `sqlx::migrate!` 路径相对于 `Cargo.toml` 所在目录，写 `src/db/migrations`
- `info!` 日志可去掉 — sqlx 内部已打印迁移日志
- `use std::path::Path` 保留 — `Database::new()` 中 `create_dir_all` 仍使用

---

## Step 3: 删除旧 `schema.sql`

**文件**：`src/db/schema.sql` — **删除**

内容已合并到 `migrations/0_initial_schema.sql`。Step 2 删除 `get_migrations()` 后无任何代码引用此文件。

---

## Step 4: 新增 `require_json` 中间件

**文件**：`src/api/middleware/mod.rs`

1. 在现有 import 块中追加：
```rust
use axum::http::{Method, header::CONTENT_TYPE};
```
`Method` 和 `CONTENT_TYPE` 加入已有的 `axum::http` 导入路径；`Request`、`Body`、`Next`、`IntoResponse`、`StatusCode` 已有。

2. 新增函数：
```rust
pub async fn require_json(
    request: Request<Body>,
    next: Next,
) -> impl IntoResponse {
    let method = request.method();
    if matches!(method, Method::GET | Method::DELETE | Method::OPTIONS | Method::HEAD) {
        return next.run(request).await;
    }
    let ct = request
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if ct.contains("application/json") {
        next.run(request).await
    } else {
        (StatusCode::UNSUPPORTED_MEDIA_TYPE, Json(json!({
            "code": 415,
            "message": "Content-Type 必须是 application/json"
        }))).into_response()
    }
}
```

> **错误格式选择**：管理端点的错误格式统一为 `{"code": N, "message": "..."}`（见 `require_admin_auth`、`ApiError`）。代理端点的错误格式为 `{"error": {"message": "...", "type": "..."}}`。中间件在路由匹配前执行、无法区分两种端点，因此使用管理端点风格（因为代理端点的前端消费者是 SDK/脚本，对 415 状态码本身就能正确处理；管理面板则依赖 `code` 字段做提示）。

> **影响范围**：所有 POST/PUT/PATCH 请求，包括 `/v1/*` 代理端点和 `/api/v1/admin/*` 管理端点。已验证 `backup/import` 使用 `Json<BackupFile>` 反序列化（即客户端发 JSON），不会被误拦。

**文件**：`src/api/router.rs`

在 `.layer()` 链中加入，代码位置在 CorsLayer 之后、TraceLayer 之前：
```rust
.layer(middleware::from_fn(crate::api::middleware::require_json))
```
最终顺序（代码自上而下）：
```rust
.layer(CorsLayer::permissive())
.layer(middleware::from_fn(crate::api::middleware::require_json))
.layer(TraceLayer::new_for_http())
```

---

## Step 5: 验证

1. `cargo check` — 编译通过，确认 `sqlx::migrate!` 宏找到迁移文件
2. `cargo test` — 现有测试通过
3. 新建测试数据库启动验证：
   ```bash
   rm -f /tmp/test_galaxy.db
   GALAXY_DB_URL=sqlite:/tmp/test_galaxy.db cargo run -- --config config.toml
   ```
   启动后检查：
   - `sqlite3 /tmp/test_galaxy.db "SELECT * FROM _sqlx_migrations;"` — 有 V0 记录且 `success` 列非空
   - 所有表存在、默认设置已插入
4. `require_json` 中间件验证：
   ```bash
   # 应返回 415
   curl -s -o /dev/null -w '%{http_code}' -X POST http://localhost:8080/api/v1/admin/auth/login \
     -H 'Content-Type: text/plain' -d '{}'
   # 应返回 200 或 401（正常处理）
   curl -s -o /dev/null -w '%{http_code}' -X POST http://localhost:8080/api/v1/admin/auth/login \
     -H 'Content-Type: application/json' -d '{"username":"x","password":"x"}'
   ```

---

## 文件变更清单

| 文件 | 操作 | 说明 |
|------|------|------|
| `src/db/migrations/1_initial_schema.sql` | **新建** | 合并所有历史迁移（重命名为 sqlx 标准格式，version ≥ 1） |
| `src/db/schema.sql` | **删除** | 已合并到迁移文件 |
| `src/db/mod.rs` | **修改** | 替换 `run_migrations()`，删除 `Migration`/`get_migrations()` 及 `include_str!` |
| `src/api/middleware/mod.rs` | **修改** | 新增 `require_json` + 追加 `Method`/`CONTENT_TYPE` import |
| `src/api/router.rs` | **修改** | 顶层加 `require_json` layer |

---

## 风险与缓解

| 风险 | 缓解 |
|------|------|
| `sqlx::migrate!` 编译期扫描目录，文件名必须符合 `{version}_{name}.sql`，且 version > 0 | 使用 `1_initial_schema.sql`，`cargo check` 即可验证 |
| 现有用户需删除旧数据库文件 | 已确认可丢弃旧数据 |
| `require_json` 对 `application/json; charset=utf-8` 等变体 | 用 `.contains("application/json")` 而非精确匹配 |
| 管理端点与代理端点错误格式不同，中间件只能选一种 | 使用管理端点风格，代理端点消费者靠 HTTP 状态码即可 |
| `backup/import` 等 POST 端点被误拦 | 已确认所有 POST 端点均使用 `application/json` |
