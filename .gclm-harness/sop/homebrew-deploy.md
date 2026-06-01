# SOP: Homebrew 本地部署排查

## 服务路径

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

## 排查顺序

1. **时间对齐** — `output.log` 使用 UTC（如 `2026-05-29T07:52:24Z` = 北京时间 `15:52:24`）
2. **查 usage_logs** — 确认 `requested_model`、`actual_model`、`channel_id`、`group_id`、`status_code`、`is_stream`、`error_message`、`response_content`、`upstream_key_hint`
3. **查 channels 表** — 用 `channel_id` 回查命中的渠道、端点、key 数量；用 `upstream_key_hint` 定位具体上游 key
4. **流式请求特殊处理** — 如果 cc 看到上游错误但 Web 日志没有失败记录，重点检查是否为流式请求

## 流式错误记录规则

- 非 流式 上游失败：同时写入 `error_message` 和 `response_content`
- 流式 SSE 建立前失败：写入 `usage_logs`
- 流式 SSE 建立后错误事件：`error_message` 写入日志，`status_code` 记为失败态（如 502）
- 首个 SSE 事件就是错误：代理可在尚未向客户端输出内容前触发 key/渠道重试
- 已输出正常内容后流内错误：只记录并透传，不触发无感 key 切换

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
