# SOP: CI 失败排查与修复

## 触发条件

GitHub Actions CI 运行失败（`push` 或 `PR` 触发），需要定位原因并修复。

## 诊断步骤

### 1. 获取失败的 run ID

```bash
# 查看最近的 CI 运行状态
gh run list --limit 5

# 或直接从 GitHub 通知 / PR 页面拿到 run URL
# https://github.com/gclm/galaxy-router/actions/runs/<RUN_ID>
```

### 2. 查看失败日志

```bash
# 只看失败 job 的日志
gh run view <RUN_ID> --log-failed

# 看完整日志（含成功 job）
gh run view <RUN_ID> --log
```

- `exit code 101` → clippy 警告被 `-D warnings` 升级为错误，或编译失败
- `exit code 1` → 测试失败
- 其他 exit code → 环境问题（依赖安装失败、缓存损坏等）

### 3. 按 job 类型定位

#### 3a. Backend — `cargo clippy -- -D warnings` 失败

```bash
# 本地复现
cargo clippy -- -D warnings
```

常见 clippy 告警：

| 规则 | 典型原因 | 修复方式 |
|------|----------|----------|
| `useless_conversion` | 多余的 `.into()` / `.to_string()` | 删除多余的转换 |
| `collapsible_match` | `match` 分支内嵌套 `if` 可合并为 guard | `Some(x) if condition =>` |
| `dead_code` | 未使用的函数/变量/导入 | 删除或加 `#[allow(dead_code)]` |
| `redundant_clone` | 不必要的 `.clone()` | 删除 |
| `needless_pass_by_value` | 函数参数可以改为引用 | 改为 `&T` |

#### 3b. Backend — `cargo test` 失败

```bash
# 本地复现
cargo test

# 只跑特定测试
cargo test <test_name> -- --nocapture
```

#### 3c. Frontend — ESLint 失败

```bash
cd web && npm run lint
```

### 4. 修复并验证

```bash
# 修复后本地验证全部检查
cargo clippy -- -D warnings && cargo test
```

### 5. 推送并确认 CI 通过

```bash
git add -A && git commit -m "fix: <描述>"
git push

# 监控新 run
gh run list --limit 1
gh run watch
```

## 修复清单

| 日期 | 问题 | 修复 |
|------|------|------|
| 2026-06-01 | `collapsible_match` + `useless_conversion` 导致 clippy 失败 | `Some("thinking") if condition =>` + 删除多余 `.into()` |
