# 验收报告 — 版本更新检查

## 验收契约核对

| # | 场景 | 期望 | 实际 | 通过 |
|---|------|------|------|------|
| 1 | 有新版 | current < latest → `has_update=true` | `handler_detects_new_version`（mock `v99.0.0`）→ `has_update=true`、`latest_version=99.0.0` | ✓ |
| 2 | 已是最新 | `has_update=false` | `handler_no_update_when_latest_older`（mock `v0.0.1`）→ `has_update=false` | ✓ |
| 3 | build metadata 不影响 | `1.0.2+meta` vs `1.0.2` → false | `has_update_false_when_equal` + `parse_strips_v_prefix_and_build_metadata` | ✓ |
| 4 | 缓存命中不发请求 | TTL 内不重打 GitHub | `cache_hit_skips_refetch`（wiremock `expect(1)`，两次调用共享 cache）| ✓ |
| 5 | GitHub 不可达 | 不 panic，返回错误 | `returns_error_when_github_fails`（mock 500）→ `Err` | ✓ |
| 6 | 版本比较纯函数 | 6 种边界 | `parse_*` / `has_update_*` / `display_*` 共 7 个纯函数单测全过（多位数 `1.10>1.9`、`v` 前缀、解析失败保守返回 false）| ✓ |

> #1/#2 用 `v99.0.0` / `v0.0.1` 规避与编译期固定版本号（`GALAXY_BUILD_VERSION`）的耦合；精确的相等/比较边界由 #6 纯函数单测覆盖。

## 测试结果

- **lint（clippy）**：✓ `update_check.rs` 无 warning（项目其他既有 warning 非本次引入，按"精准修改"未动）
- **type-check（前端 tsc）**：✓
- **后端 test**：✓ `update_check` 11 passed；全量 `cargo test` 14 个 test suite 全部 `0 failed`（无回归）
- **前端 build**：✓ vite build 成功（chunk size warning 为项目既有，非本次引入）

## 挂载点检查

| 文件 | 状态 |
|---|---|
| `src/api/handlers/admin/update_check.rs` | ✓ 新建：state + handler + 版本比较 + TTL 缓存 + 降级 + 常量 + 11 测试 |
| `src/api/handlers/admin/mod.rs` | ✓ `pub mod update_check;` |
| `src/api/router.rs` | ✓ import + `update_check_state` 构建 + `GET /api/v1/admin/update-check` 挂载 |
| `frontend/src/api/types.ts` | ✓ `UpdateCheck` 类型 |
| `frontend/src/api/query-hooks.ts` | ✓ `useUpdateCheck`（1h 轮询 + `refetchOnMount:'always'` + `retry:false`）|
| `frontend/src/components/UpdateCheckDialog.tsx` | ✓ 新建：四态（检查中 / 发现新版本 / 已是最新 / 检查失败）|
| `frontend/src/components/layout/Header.tsx` | ✓ 主题切换左侧 `Download` 图标 + `has_update` 红点 + 挂载弹窗 |

## 与参考实现的差异（落地确认）

| 维度 | octopus | galaxy-router（本次）|
|---|---|---|
| 更新动作 | 自动覆盖二进制 + 重启 | 仅检查 + 提示（适配 Homebrew/Docker/裸二进制）|
| 版本比较 | 前端字符串 `!==` | 后端 `Vec<u64>` semver（处理 build metadata）|
| 缓存 | 无服务端缓存 | 服务端 10min 内存缓存（国内网络 + 限流）|
| 网络降级 | 直连失败→显式代理重试 | reqwest 默认读 `HTTPS_PROXY`，10s 超时降级 |

## UI 手动验证

自动化已覆盖逻辑（11 后端测试）与渲染编译（前端 build + tsc）。建议在真实环境手动确认（Gate 3 用户确认范畴）：

- 顶栏主题切换左侧出现 `Download` 图标
- 有新版本时图标右上角红点
- 点击打开弹窗：当前/最新版本 + release notes + 「前往 GitHub 下载」
- GitHub 不可达时弹窗显示「检查失败」+ HTTPS_PROXY 提示

## v2 变更：下载镜像 fallback（最终实现）

经用户反馈（国内 `api.github.com` 检查失败）+ 参考 opskat `internal/update_svc`，v2 改为三级 fallback。

### 检查逻辑（三级）
1. 默认 `https://api.github.com/repos/{github.repo}/releases/latest`（配了 `proxy.url` 走代理，复用 `relay/state.rs` 模式）
2. 失败 + 配了 `update.mirror` → 镜像下载 `release-info.json`（ghfast/gh-proxy 加速 github.com 下载）
3. 都失败 → `ApiError`（前端「检查失败」）

### 新增挂载点
| 文件 | 改动 |
|---|---|
| `src/db/migrations/13_add_github_repo_setting.sql` | 新建：`github.repo` + `update.mirror` 配置 |
| `.github/workflows/release.yml` | 每次 release 生成 `release-info.json` 作为 asset（Generate 在 Create 之前）|
| `src/api/handlers/admin/update_check.rs` | `mirror` 字段 + 三级 fallback + `apply_mirror` + `from_pool` 读 proxy/repo/mirror |
| `src/api/handlers/admin/settings.rs` | 白名单加 `github.repo` / `update.mirror` |
| `frontend/src/pages/Settings.tsx` | 新增「版本更新」分类（GitHub 仓库 + 下载镜像）|

### 测试
- 新增 `mirror_fallback_when_api_fails`：api 500 → 镜像 `release-info.json` 成功
- `update_check` 共 12 测试全过；全量 14 suite 无回归；clippy / tsc / vite build 全绿

### 用户配置（解决检查失败，二选一）
- **镜像**：设置 → 版本更新 → 下载镜像 填 `https://ghfast.top/`（或其他 github.com 加速前缀）
- **代理**：设置 → 上游代理 → 启用 + 填代理地址（检查走代理）
- 两者其一即可；配代理走代理直连 api，没代理配镜像走 fallback

## 顺手发现

无。
