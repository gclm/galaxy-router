---
status: approved
task: 2026-06-16-版本更新检查
---

# 版本更新检查 — 设计

> 参考实现：`/Users/gclm/workspace/refs/octopus/internal/update/`（Go）。本文档只取其"检查"部分，"自动更新"部分（覆盖二进制+重启）因 galaxy-router 多部署方式不适用。

## 1. 决策与约束

**用户目标**：管理员在管理后台知道 galaxy-router 是否有新版本，看到新版本的 release notes 和下载入口，自行决定何时升级。

**核心行为**
- 管理后台「系统信息」区展示：当前版本 / 最新版本 / 是否有更新
- 有新版本：展示 release notes + GitHub release 链接 + 下载产物链接（zip / checksums）
- 服务端短期缓存检查结果（默认 10 分钟）：缓存命中秒回、不打 GitHub、不被限流
- 进入页面自动检查一次 + 手动「检查更新」按钮 + 前端每小时轮询（react-query）

**成功标准**
- 落后于最新 → `has_update: true`（semver 比较，正确处理 build metadata 与 `v` 前缀）
- 已是最新 → `has_update: false`
- GitHub 不可达 → 不 panic、不阻塞，前端展示「检查失败，稍后再试」
- 缓存有效期内 → 不发外部 HTTP

**明确不做（v1）**
- ❌ 自动下载/覆盖二进制/重启（octopus 的 `UpdateCore`）——Homebrew 路径被 brew 管理、Docker 容器无法自更新，风险高
- ❌ 后台定时轮询任务（用前端 react-query `refetchInterval` 代替，零后端复杂度）
- ❌ 过期缓存兜底（缓存过期 + 请求失败 = 直接报错，简单优先，v2 再加"返回上次缓存"）
- ❌ 双链路"直连失败→显式代理重试"（reqwest 默认已读 `HTTPS_PROXY`，覆盖该场景，无需额外代码）
- ❌ 持久化检查结果到数据库（纯内存缓存，重启即失效，够用）
- ❌ 引入 `semver` crate（版本号是简单 `x.y.z`，`Vec<u64>` 逐段比较即可）

## 2. 名词层 + 编排层

### 现状
- 版本来源：`env!("GALAXY_BUILD_VERSION")`（`build.rs` 注入），形如 `1.0.2+homebrew.20260611`，`+` 后是 build metadata
- GitHub 发布：tag `v{version}`（如 `v1.0.2`），`release.yml` 监听 `v*` tag 触发；产物 `galaxy-router-{platform}.zip` + `checksums-sha256.txt`
- 版本展示现状：`system_info.rs::get` 返回 `SystemInfo.version`，前端 `useSystemInfo()`（queryKey `['system-info']`）在 Dashboard 消费
- HTTP 客户端参考：`router.rs` 多处 `reqwest::Client::builder().timeout(N).build().expect(...)`（http_client 30s / fetch_client 10s）
- 前端有 `@tanstack/react-query`，支持 `refetchInterval` / `refetchOnMount`
- GitHub API（免 token）：`GET https://api.github.com/repos/gclm/galaxy-router/releases/latest` → `{tag_name, name, html_url, body, published_at, assets}`

### 变化

**后端新增**
- `src/api/handlers/admin/update_check.rs`
  - `UpdateCheckState { http_client: reqwest::Client, cache: Arc<RwLock<Option<Cached>>> }`
  - `get()`：缓存未过期→返回缓存；过期→调 GitHub latest release，比较版本，写缓存，返回；请求失败→返回 `ApiError`
  - `parse_version(s) -> Option<Vec<u64>>`：去 `v` 前缀 → 去 `+` build metadata → 按点分段解析
  - `has_update(current, latest) -> bool`：`Vec<u64>` 逐段比较（latest > current），解析失败保守返回 `false`
- `src/api/handlers/admin/mod.rs`：`pub mod update_check;`
- `src/api/router.rs`：import + 构建 `update_check_client`（timeout 10s）+ `update_check_state` + 独立 nest `GET /api/v1/admin/update-check`
- 常量（`update_check.rs` 内）：`GITHUB_LATEST_API` / `UPDATE_CHECK_TTL_SECS = 600` / `UPDATE_CHECK_TIMEOUT_SECS = 10` / `HTTP_TIMEOUT_SECS = 10`

**响应结构**（`ApiResponse<UpdateCheckResponse>`，约束 #2 统一格式）
```rust
struct UpdateCheckResponse {
    current_version: String,  // 纯 x.y.z（去 metadata）
    latest_version: String,   // 去 v 前缀
    has_update: bool,
    release_url: String,      // html_url
    release_notes: String,    // body（release notes）
    published_at: String,     // ISO8601
    checked_at: i64,          // 本次/上次检查的 unix 秒（缓存命中时是上次）
}
```

**前端新增**（入口 = 顶栏「更新」图标 + 详情 Dialog，全局可见）
- `frontend/src/api/types.ts`：`UpdateCheck` 类型（对应 `UpdateCheckResponse`）
- `frontend/src/api/query-hooks.ts`：`useUpdateCheck()`（queryKey `['update-check']`，`refetchInterval: 3600000`，`refetchOnMount: 'always'`）
- `frontend/src/components/UpdateCheckDialog.tsx`（新建，仿 `TestModelDialog.tsx`，复用 `components/ui/dialog`）：详情弹窗。有更新 →「发现新版本 vX.Y.Z / 当前 vX.Y.Z」+ release notes（纯文本 `whitespace-pre-wrap`，**不**做 markdown 渲染）+ GitHub Release 链接；无更新 →「已是最新」+ 当前版本
- `frontend/src/components/layout/Header.tsx`：主题切换按钮**左侧**新增「更新」图标按钮（lucide `Download`）。`has_update=true` → 图标右上角红点（`absolute` 小红圆点）；点击打开 `UpdateCheckDialog`
- Dashboard 系统信息卡片**不动**（版本格保持现状，入口只在顶栏）
- 下载链接直接指向 GitHub Release 页面（`release_url`），**不**自动判断用户平台选 zip（简单优先）

### 挂载点清单
| 文件 | 改动 |
|---|---|
| `src/api/handlers/admin/update_check.rs` | 新建（state + handler + 版本比较 + 常量）|
| `src/api/handlers/admin/mod.rs` | `pub mod update_check;` |
| `src/api/router.rs` | import + 构建 client/state + nest 路由 |
| `frontend/src/api/types.ts` | `UpdateCheck` 类型 |
| `frontend/src/api/query-hooks.ts` | `useUpdateCheck()` |
| `frontend/src/components/UpdateCheckDialog.tsx` | 新建：版本详情弹窗 |
| `frontend/src/components/layout/Header.tsx` | 主题切换左侧加「更新」图标按钮（红点 + 打开弹窗）|

### 卸载（想拔掉要动哪些）
删 `update_check.rs` + `mod.rs` 声明 + `router.rs` 的 import/state/路由 + 前端类型/hook/区块。**无数据库迁移、无配置项** → 卸载干净。

## 3. 验收契约

| # | 场景 | 输入 | 期望 | 通过 |
|---|---|---|---|---|
| 1 | 有新版 | current `1.0.2+homebrew.x`，mock tag `v1.0.3` | `has_update=true, current_version=1.0.2, latest_version=1.0.3` | ☐ |
| 2 | 已是最新 | current `1.0.2`，mock tag `v1.0.2` | `has_update=false` | ☐ |
| 3 | build metadata 不影响 | current `1.0.2+homebrew.x`，tag `v1.0.2` | `has_update=false` | ☐ |
| 4 | 缓存命中不发请求 | TTL 内第二次请求 | mock 只被调用一次，返回同结果 | ☐ |
| 5 | GitHub 不可达 | mock 连接错误 | `ApiError`（不 panic），前端展示「检查失败」 | ☐ |
| 6 | 版本比较纯函数 | `has_update("1.0.2","1.0.3")=true` / `("1.0.2","1.0.2")=false` / `("1.0.2+meta","1.0.2")=false` / `("1.9.0","1.10.0")=true`（多位数）/ `("1.0.2","v1.0.3")=true`（v 前缀） | 单测全过 | ☐ |

外部 HTTP 全部 mock（遵循 conventions.md「外部 HTTP 请求全部 mock，不依赖真实网络」）。

## 4. 推进策略

**阶段 A — 版本比较纯函数 + 单测**
- 实现 `parse_version` / `has_update`，覆盖验收契约 #6 全部边界（含多位数 1.10>1.9、build metadata、v 前缀）
- 退出信号：`cargo test has_update` 绿

**阶段 B — 后端 handler + GitHub mock 集成测试**
- 实现 `UpdateCheckState` + `get()` + TTL 缓存，用 wiremock 模拟 GitHub API，覆盖验收契约 #1–#5
- 退出信号：`make test-api` 绿

**阶段 C — 前端类型 + hook + 顶栏入口 + 弹窗**
- 加 `UpdateCheck` 类型、`useUpdateCheck()`、`UpdateCheckDialog`、Header 主题切换左侧的更新图标按钮（`has_update` 红点 + 点击打开弹窗）
- 退出信号：`make build` 绿 + 手动验证：有更新→红点+弹窗内容正确；无更新→不亮+弹窗显示「已是最新」

**阶段 D — 缓存与降级验证**
- 集成测试覆盖：缓存命中不发请求（#4）、GitHub 不可达优雅降级（#5）
- 退出信号：对应集成测试绿

---

## v2 变更：下载镜像 fallback（解决国内 api.github.com 检查失败）

参考 opskat `internal/update_svc`。原方案依赖 api.github.com 可达（国内常失败），v2 加镜像 fallback。

### 新增
- **CI 生成 `release-info.json`**：`release.yml` 每次 release 生成（tag_name/name/body/html_url/published_at），作为 asset。镜像 fallback 下载它。
- **settings `update.mirror`**：下载镜像前缀（如 `https://ghfast.top/`），空=不启用 fallback。
- **settings `github.repo`**：owner/repo，拼 api/镜像 URL，便于 fork 自定义。

### 检查逻辑（三级 fallback）
1. **默认**：`GET https://api.github.com/repos/{github.repo}/releases/latest`（走代理如果 `proxy.url` 配了；复用 relay/state.rs 的 proxy 构建模式）
2. **失败 + 配了 mirror**：`GET {mirror}https://github.com/{github.repo}/releases/latest/download/release-info.json`（镜像加速下载静态 JSON，ghfast/gh-proxy 支持 github.com 下载）
3. **都失败**：返回 ApiError，前端显示「检查失败」

### release-info.json schema（与 GithubRelease 兼容，复用解析）
```json
{"tag_name":"v1.0.3","name":"v1.0.3","body":"...","html_url":"https://github.com/.../releases/tag/v1.0.3","published_at":"2026-..."}
```

### 不变
- 版本比较 semver（处理 build metadata）、服务端 10min 缓存、前端顶栏入口 + 弹窗（提示版本 + GitHub 链接）
- 弹窗不做"下载镜像"——只给 GitHub release 链接（用户自行下载），镜像仅用于后端检查 fallback
