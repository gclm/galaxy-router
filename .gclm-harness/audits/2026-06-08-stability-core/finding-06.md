---
doc_type: audit-finding
id: F-06
nature: maintainability
severity: P2
confidence: high
suggested_action: cs-refactor
---

# 队列单测依赖 5 秒真实超时，CI 稳定性差

## 证据

`src/proxy/queue.rs:54-60`：

```rust
let q = RequestQueue::new(2, 5);
let p1 = q.acquire().await.unwrap();
let p2 = q.acquire().await.unwrap();
assert!(matches!(q.acquire().await, Err(QueueError::QueueFull { .. })));
```

第三次 `acquire()` 必须等 `timeout_secs=5` 才返回错误。

## 为什么是问题

这个测试正常通过也要至少等待 5 秒。CI 上如果多次运行或与其它慢测试叠加，会显著拉长反馈；开发者也容易误判为测试卡死。

## 影响

- `make test` 反馈慢，降低修复效率。
- 真实超时依赖让测试更脆弱。

## 建议

将测试 timeout 改为几十毫秒，或使用 `tokio::time::pause/advance` 控制虚拟时间。队列功能测试不应依赖秒级真实等待。
