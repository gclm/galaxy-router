---
doc_type: audit-finding
id: F-02
nature: bug
severity: P1
confidence: high
suggested_action: cs-issue
---

# 新增分组项后未失效分组缓存，新路由可能不生效

## 证据

`src/api/handlers/admin/groups.rs:455-477` 新增 `group_items` 后直接返回：

```rust
.execute(&state.pool).await?;
let item = GroupItem { ... };
Ok((StatusCode::CREATED, Json(ApiResponse::success(item))))
```

而 `create`、`update`、`delete`、`delete_item` 路径都调用了 `state.cache.invalidate_all_groups().await`。

## 为什么是问题

分组缓存 `ProxyCache.groups` 按分组名缓存 `GroupInfo`，里面包含 items。新增分组项若不失效，已缓存分组不会包含新渠道，代理路由继续使用旧候选集。

## 影响

- 管理端新增 fallback 渠道后，实际代理不走新增渠道。
- 用户以为扩容/容灾已生效，但高峰或故障时仍按旧配置运行。

## 建议

`add_item` 成功后调用 `state.cache.invalidate_all_groups().await`。如后续支持单分组失效，可新增 `invalidate_group(name/id)`。
