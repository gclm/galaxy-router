# SOP: Homebrew 本地部署排查

## 触发条件

服务异常、请求报错、需要查看生产日志或部署新版本时。

## 诊断步骤

### 1. 确认服务路径

```bash
plutil -p ~/Library/LaunchAgents/homebrew.mxcl.galaxy-router.plist
```

| 资源 | 路径 |
|------|------|
| 配置文件 | `/opt/homebrew/etc/galaxy-router/config.toml` |
| 工作目录 | `/opt/homebrew/var/lib/galaxy-router` |
| 数据库 | `/opt/homebrew/var/lib/galaxy-router/galaxy.db` |
| 输出日志 | `/opt/homebrew/var/log/galaxy-router/output.log` |
| 错误日志 | `/opt/homebrew/var/log/galaxy-router/error.log` |
| 二进制 | `/opt/homebrew/opt/galaxy-router/bin/galaxy-router` |

> 不要默认查看仓库内的 `data/galaxy.db`，以 launchd 配置为准。

### 2. 时间对齐后查日志

`output.log` 使用 UTC（如 `2026-05-29T07:52:24Z` = 北京时间 `15:52:24`），用报错时间戳换算后再查。

### 3. 查 usage_logs 定位问题请求

```bash
sqlite3 /opt/homebrew/var/lib/galaxy-router/galaxy.db \
  "SELECT created_at, requested_model, status_code, is_stream,
          error_message, upstream_key_hint
   FROM usage_logs ORDER BY created_at DESC LIMIT 10;"
```

- `status_code != 200` → 请求失败，查 `error_message`
- `status_code = 200` 但 `error_message` 非空 → 流式 SSE 内错误（status 被代理记为 200 但实际有问题）
- → 进入步骤 4

### 4. 用 channel_id 回查渠道配置

```bash
sqlite3 /opt/homebrew/var/lib/galaxy-router/galaxy.db \
  "SELECT id, name, endpoints, api_keys FROM channels WHERE id='<CHANNEL_ID>';"
```

用 `upstream_key_hint` 定位具体使用了哪个上游 key。

### 5. 判断流式 vs 非流式错误记录规则

- **非流式**上游失败：同时写入 `error_message` 和 `response_content`
- **流式 SSE 建立前**失败：写入 `usage_logs`
- **流式 SSE 建立后**错误事件：`error_message` 写入日志，`status_code` 记为失败态（如 502）
- 首个 SSE 事件就是错误 → 可触发 key/渠道重试
- 已输出正常内容后的流内错误 → 只记录并透传，不触发无感切换

## 部署新版本

```bash
make brew-deploy          # 构建 + 备份旧二进制 + 部署 + 自动重启
make brew-deploy BREW_RESTART=0  # 只部署不重启
make brew-restart         # 单独重启服务
```

环境变量覆盖：
- `BREW_BIN` — 目标二进制路径（默认 `/opt/homebrew/opt/galaxy-router/bin/galaxy-router`）
- `BREW_SERVICE` — Homebrew 服务名（默认 `gclm/tap/galaxy-router`）
- `BREW_RESTART` — 是否自动重启（默认 `1`）
