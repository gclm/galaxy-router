# 常见坑

> 硬约束和必须知道的陷阱

## 硬约束（不许做的事）

- 错误类型已统一到 `src/error/` 包：`error::proxy`（ProxyError/ErrorClass/ErrorFormat）、`error::app`（ApiError/ApiResponse）
- `protocol/inbound/` 和 `protocol/outbound/` 不是空目录，已有协议转换实现，可以修改
- 代理 API 响应不能包装成 `{code, message, data}`——必须透传上游原格式，保证 SDK 兼容
- 数据库迁移不可回滚、不可修改已发布版本，只能追加新文件（version > 0）
- 业务配置变更（渠道/分组）后必须调用 `cache.invalidate()` 同步 `ProxyCache`

## 环境设置（先 X 才能 Y）

- 先 `make build`（含前端）才能 `make run`
- 测试用临时数据库在 `/tmp/galaxy_test_*`，测试自动创建和清理
- **Homebrew 部署分两种情况**：
  - 首次部署：`brew install gclm/tap/galaxy-router`，然后 `make brew-deploy` 覆盖二进制
  - 更新部署：直接 `make brew-deploy`（会自动备份旧二进制 + 重启服务）
- `make test` 只跑单元/集成测试（临时数据库 `/tmp/galaxy_test_*`），没有真实数据端到端测试——后续需新增用 `data/galaxy.db` 的 E2E 测试

## 常见坑

- **协议转换 null 字段**：上游返回的 JSON 可能有 null 字段，转换时需要显式处理（`068741f` 修过）
- **流式 tool_calls 碎片化**：SSE 流中 tool_calls 分片到达，拼接日志时需要缓冲（`54f4802` 修过）
- **上游 Key 余额不足但返回 5xx**：`classify_upstream` 会检查 body 关键词（"insufficient_quota" 等）来识别 Key 问题，新增 Key 相关错误时需要同步更新 `KEY_NEEDLES`
- **`sqlx::migrate!()` 宏在编译期扫描 `src/db/migrations/`**：路径是相对 `Cargo.toml` 的，文件名格式 `{version}_{name}.sql`，version 必须递增
- **`HeaderValue::from_str` 会 panic**：用户输入作为 header value（如 API Key）时，必须先 `validate_header_value()` 校验，否则 CRLF / 控制字符会 panic
- **`models` 字段解析**：`parse_models` 用 `serde_json::from_str`，空字符串和非法 JSON 返回空 Vec，不要假设一定有值
- **`ProxyCache` 双层结构**：`groups`、`channels`、`compiled_regex` 各有独立缓存，失效时需要逐个处理

## 调试技巧

- 查看请求日志：`usage_logs` 表记录了每次请求的 input/output tokens、cost、latency、attempt stats
- 查看调度决策：`scheduler/trace.rs` 有 trace 日志，关注 `score` 和 `selection` 相关 tracing
- 测试单个模块：`cargo test --lib relay::pipeline` 跑子模块测试
- 健康检查：`GET /api/v1/health` 返回 `needs_setup` 判断是否需要初始化
