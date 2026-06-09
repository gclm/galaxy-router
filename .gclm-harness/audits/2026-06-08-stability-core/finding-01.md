---
doc_type: audit-finding
id: F-01
nature: bug
severity: P1
confidence: high
suggested_action: cs-issue
---

# 渠道更新后未失效 ProxyCache，代理可能继续使用旧上游配置

## 证据

`src/api/handlers/admin/channels/crud.rs:280-287`：

```rust
builder.build().execute(&state.pool).await?;
let channel = get_channel_by_id(&state.pool, &id, state.timezone_offset).await?;
Ok(Json(ApiResponse::success(channel)))
```

同文件创建/删除路径分别有失效逻辑：`create` 调用 `invalidate_all_channels()`，`delete` 调用 `invalidate_channel(&id)`。

## 为什么是问题

项目硬约束要求业务配置变更后必须同步失效对应 `ProxyCache`。更新渠道时可能改 `api_keys`、`endpoints`、`models`、`timeout_secs`、`max_concurrency`、`enabled` 等代理核心字段，但缓存不失效会导致运行时仍使用旧配置。

## 影响

- 更新 API Key 后代理仍拿旧 key 请求上游。
- 禁用渠道或修改 endpoint 后仍可能继续转发到旧渠道/旧地址。
- 修改模型列表后模型索引不一致。

## 建议

更新成功后调用 `state.cache.invalidate_channel(&id).await`；如果 `models` 改动可能影响反向索引，单渠道失效已足够。补集成测试覆盖“先缓存渠道 → 更新 → 再选择/读取必须命中新配置”。
