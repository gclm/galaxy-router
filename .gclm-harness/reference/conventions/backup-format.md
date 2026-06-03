# Galaxy Router 数据备份格式规范

## 概述

Galaxy Router 支持通过 JSON 文件进行配置数据的导入和导出。备份文件包含渠道、分组、API Key 和系统设置，不包含用户账户、使用日志和统计数据。

## 导出格式

```jsonc
{
  // 固定标识，用于校验文件类型
  "format": "galaxy-router-backup",
  // 格式版本，当前为 1
  "version": 1,
  // 导出时间（RFC 3339）
  "exported_at": "2026-05-28T12:30:00+08:00",
  // 应用版本
  "app_version": "0.0.1",
  // 业务数据
  "data": {
    "channels": [ /* 见下方渠道结构 */ ],
    "groups": [ /* 见下方分组结构 */ ],
    "api_keys": [ /* 见下方 API Key 结构 */ ],
    "settings": [ /* 见下方设置结构 */ ]
  }
}
```

## 数据结构

### 渠道（Channel）

```json
{
  "id": "019xxx",                    // 原始 ID（仅参考，导入时重新生成）
  "name": "OpenAI 官方",            // 渠道名称（唯一，导入时跳过同名渠道）
  "api_keys": [                      // 上游 API Key 列表（原始值）
    "sk-xxxx"
  ],
  "endpoints": [                     // 端点配置
    {
      "type": "openai_chat",         // 端点类型枚举
      "base_url": "https://api.openai.com/v1"
    }
  ],
  "models": [                        // 支持的模型列表
    "gpt-4o",
    "gpt-4o-mini"
  ],
  "rate_limit_rpm": null,            // 每分钟请求限制
  "rate_limit_tpm": null,            // 每分钟 Token 限制
  "failure_threshold": 3,            // 故障阈值
  "blacklist_minutes": 10,           // 黑名单时长（分钟）
  "concurrency": 10,                 // 并发限制
  "enabled": true                    // 是否启用
}
```

**端点类型枚举值**：

| 值 | 说明 |
|---|---|
| `openai_chat` | OpenAI Chat Completions |
| `openai_response` | OpenAI Responses |
| `anthropic` | Anthropic Messages |
| `gemini` | Google Gemini |
| `openai_embedding` | OpenAI Embeddings |
| `openai_images` | OpenAI Images |

### 分组（Group）

```json
{
  "name": "GPT-4o 分组",             // 分组名称（唯一）
  "match_regex": null,               // 模型匹配正则（null 表示精确匹配）
  "retry_enabled": true,             // 是否启用重试
  "max_retries": 3,                  // 最大重试次数
  "first_token_timeout_secs": 30,    // 首 Token 超时（秒）
  "enabled": true,                   // 是否启用
  "items": [                         // 分组子项
    {
      "channel_name": "OpenAI 官方",  // 通过渠道名称关联（非 ID）
      "model_name": "gpt-4o",        // 模型名称
      "priority": 1,                 // 优先级
      "weight": 100                  // 权重
    }
  ]
}
```

**设计说明**：分组子项使用 `channel_name` 而非 `channel_id` 关联渠道，因为导入时渠道 ID 会重新生成。导入时会按名称查找渠道，找不到则跳过该子项。

### API Key

```json
{
  "id": "019xxx",                    // 原始 ID（仅参考）
  "name": "生产环境",                // Key 名称
  "api_key": "gp-019xxx",           // 完整 Key 值（可被导入恢复）
  "enabled": true                    // 是否启用
}
```

### 设置（Setting）

```json
{
  "key": "scheduler.top_k",         // 设置项 Key
  "value": "7"                       // 设置项值（字符串）
}
```

导入时按 key 匹配已有设置项进行更新，不会创建数据库中不存在的设置项。

## 导出范围

| 数据类型 | 导出 | 导入行为 |
|---------|------|---------|
| 渠道（channels） | ✅ | 同名渠道跳过（INSERT OR IGNORE） |
| 分组（groups） | ✅ 含子项 | 同名分组跳过，子项通过渠道名称关联 |
| API Key | ✅ | 同 api_key 值跳过 |
| 系统设置 | ✅ | 按 key 更新已有设置项 |
| 用户账户 | ❌ | 不导出，含密码哈希安全风险 |
| 使用日志 | ❌ | 数据量大，不属于配置 |
| 每日统计 | ❌ | 数据量大，不属于配置 |
| 模型定价 | ❌ | 可从外部数据源刷新 |

## 导入流程

1. 校验 `format` 字段为 `galaxy-router-backup`，`version` 为 `1`
2. 按顺序导入：渠道 → API Key → 设置 → 分组
   - 渠道先导入，确保分组子项可以通过渠道名称找到对应 ID
   - API Key 先于设置导入（无依赖关系，但保持一致顺序）
3. 每条记录独立处理，单条失败不影响其余记录
4. 返回导入结果统计（成功数 + 错误列表）

## API 端点

### 导出

```
GET /api/v1/admin/backup/export
Authorization: Bearer <token>
```

返回完整的 `BackupFile` JSON。

### 导入

```
POST /api/v1/admin/backup/import
Authorization: Bearer <token>
Content-Type: application/json

{ BackupFile JSON }
```

返回导入结果：

```json
{
  "code": 0,
  "data": {
    "channels_imported": 5,
    "groups_imported": 3,
    "api_keys_imported": 2,
    "settings_imported": 7,
    "errors": ["渠道 'xxx': 已存在"]
  }
}
```

## 前端交互

设置页面新增"数据备份"标签页：

- **导出**：点击按钮下载 JSON 文件，文件名格式 `galaxy-router-backup-20260528.json`
- **导入**：选择文件上传，显示导入结果（成功数 + 错误列表）
