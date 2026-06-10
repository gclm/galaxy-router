# 仓库表面 — API 路由清单

来源：`src/api/router.rs`

## 代理 API（`/v1/*`，透传协议格式，API Key 认证）

| 方法 | 路径 | Handler | 说明 |
|------|------|---------|------|
| POST | `/v1/chat/completions` | `chat::proxy` | OpenAI 聊天补全 |
| POST | `/v1/responses` | `responses::proxy` | OpenAI Responses API |
| POST | `/v1/messages` | `messages::proxy` | Anthropic Messages |
| POST | `/v1/embeddings` | `embeddings::proxy` | Embeddings |
| POST | `/v1/images/generations` | `images::proxy` | 图片生成 |
| GET  | `/v1/models` | `models::list` | 模型列表 |

## 管理 API（`/api/v1/admin/*`，JWT 认证，统一 `{code, message, data}` 格式）

### 认证（`/auth`）

| 方法 | 路径 | 说明 |
|------|------|------|
| GET  | `/api/v1/admin/auth/me` | 当前用户信息 |
| PUT  | `/api/v1/admin/auth/password` | 修改密码 |

### 渠道（`/channels`）

| 方法 | 路径 | 说明 |
|------|------|------|
| GET  | `/api/v1/admin/channels` | 渠道列表 |
| POST | `/api/v1/admin/channels` | 创建渠道 |
| GET  | `/api/v1/admin/channels/{id}` | 渠道详情 |
| PUT  | `/api/v1/admin/channels/{id}` | 更新渠道 |
| DELETE | `/api/v1/admin/channels/{id}` | 删除渠道 |
| POST | `/api/v1/admin/channels/{id}/test` | 测试渠道 |
| POST | `/api/v1/admin/channels/{id}/detect` | 检测渠道特性 |

### 分组（`/groups`）

| 方法 | 路径 | 说明 |
|------|------|------|
| GET  | `/api/v1/admin/groups` | 分组列表 |
| POST | `/api/v1/admin/groups` | 创建分组 |
| GET  | `/api/v1/admin/groups/{id}` | 分组详情 |
| PUT  | `/api/v1/admin/groups/{id}` | 更新分组 |
| DELETE | `/api/v1/admin/groups/{id}` | 删除分组 |
| POST | `/api/v1/admin/groups/{id}/items` | 添加分组项 |
| DELETE | `/api/v1/admin/groups/{id}/items/{item_id}` | 删除分组项 |

### API Keys（`/api-keys`）

| 方法 | 路径 | 说明 |
|------|------|------|
| GET  | `/api/v1/admin/api-keys` | Key 列表 |
| POST | `/api/v1/admin/api-keys` | 创建 Key |
| GET  | `/api/v1/admin/api-keys/{id}` | Key 详情 |
| PUT  | `/api/v1/admin/api-keys/{id}` | 更新 Key |
| DELETE | `/api/v1/admin/api-keys/{id}` | 删除 Key |

### 统计（`/stats`）

| 方法 | 路径 | 说明 |
|------|------|------|
| GET  | `/api/v1/admin/stats/overview` | 总览 |
| GET  | `/api/v1/admin/stats/models` | 模型统计 |
| GET  | `/api/v1/admin/stats/channels` | 渠道统计 |
| GET  | `/api/v1/admin/stats/daily` | 每日统计 |
| GET  | `/api/v1/admin/stats/api-keys` | Key 统计 |
| GET  | `/api/v1/admin/stats/latency` | 延迟统计 |
| GET  | `/api/v1/admin/stats/budgets` | 预算列表 |
| POST | `/api/v1/admin/stats/budgets` | 设置预算 |
| DELETE | `/api/v1/admin/stats/budgets/{id}` | 删除预算 |
| GET  | `/api/v1/admin/stats/logs` | 日志列表 |
| GET  | `/api/v1/admin/stats/logs/models` | 模型日志 |
| GET  | `/api/v1/admin/stats/logs/{id}` | 日志详情 |

### 模型信息（`/models/info`）

| 方法 | 路径 | 说明 |
|------|------|------|
| GET  | `/api/v1/admin/models/info` | 模型信息列表 |
| PUT  | `/api/v1/admin/models/info` | 更新模型信息 |
| GET  | `/api/v1/admin/models/info/{model}` | 单模型信息 |

### 系统 & 设置 & 备份

| 方法 | 路径 | 说明 |
|------|------|------|
| GET  | `/api/v1/admin/system-info` | 系统信息 |
| GET  | `/api/v1/admin/settings` | 设置列表 |
| GET  | `/api/v1/admin/settings/infra` | 基础设施设置 |
| PUT  | `/api/v1/admin/settings/{key}` | 更新设置 |
| GET  | `/api/v1/admin/backup/export` | 导出备份 |
| POST | `/api/v1/admin/backup/import` | 导入备份 |
| POST | `/api/v1/admin/backup/reset` | 重置数据 |
| POST | `/api/v1/admin/fetch-models` | 从上游拉取模型 |

## 公开 API（无需认证）

| 方法 | 路径 | 说明 |
|------|------|------|
| GET  | `/api/v1/health` | 健康检查（含 needs_setup） |
| POST | `/api/v1/init` | 初始化（创建管理员） |
| POST | `/api/v1/admin/auth/login` | 登录 |
