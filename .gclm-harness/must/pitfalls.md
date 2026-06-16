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
- **国内上游限流常以中文 body + 非 429 返回**：智谱 GLM Coding Plan 的 1302 限流藏在响应体内（流式 SSE 分支 `stream_executor.rs:272` 还会把它标成 502），只能靠 `KEY_NEEDLES` 的中文关键词（"速率限制"/"频率限制"）识别为 `KeyRetryable` 触发换 key。新增上游时按语义族（限流/额度/鉴权）覆盖中英文，不要只按上游枚举
- **`sqlx::migrate!()` 宏在编译期扫描 `src/db/migrations/`**：路径是相对 `Cargo.toml` 的，文件名格式 `{version}_{name}.sql`，version 必须递增
- **`HeaderValue::from_str` 会 panic**：用户输入作为 header value（如 API Key）时，必须先 `validate_header_value()` 校验，否则 CRLF / 控制字符会 panic
- **`models` 字段解析**：`parse_models` 用 `serde_json::from_str`，空字符串和非法 JSON 返回空 Vec，不要假设一定有值
- **`ProxyCache` 双层结构**：`groups`、`channels`、`compiled_regex` 各有独立缓存，失效时需要逐个处理
- **（前端）`navigator.clipboard` 受 Secure Context 限制**：只在 HTTPS/localhost/file:// 下可用,HTTP 部署下为 `undefined`,裸调 `.writeText` 会抛 `Cannot read properties of undefined`。前端复制统一走 `frontend/src/lib/utils.ts::copyText`(已含 `execCommand` 降级),勿裸调
- **（前端）API Key 预算是独立实体**：`budget_limits` 需 `api_key_id` 才能写入,create api key 接口不带预算。前端创建时设预算必须两步——先 `createApiKey` 拿到 `key.id` → `onSuccess` 里调 `setBudgetMutation`(见 `ApiKeys.tsx::handleCreate`)
- **按时间范围查 `created_at` 别套函数**：`WHERE date(datetime(created_at,'+8h')) >= ?` 会令 `idx_usage_logs_created_at` 失效、全表扫(仪表盘曾因此卡 pending)。改用 UTC 范围裸列 `created_at >= ? AND created_at < ?`(helper `metrics/query/mod.rs::range_utc_days/between`)。同理 `check_budget` 月消费须按当月过滤,否则全部历史计入→"一设就被拦"
- **chrome-mcp `fill` 对 React 受控 input 无效**：只改 DOM value,不触发 React `onChange`(valueTracker 不感知),submit 仍发旧值。测 React 表单清空/改值要用 `evaluate_script` 走 native setter + `dispatchEvent(new Event('input',{bubbles}))`
- **SQLite 聚合 `SUM` 返回类型不固定**：全 INTEGER 输入时返回 INTEGER,含 REAL 才返回 REAL。sqlx 按 `f64` 解码 INTEGER 会报 "not compatible with SQL INTEGER"(曾让 `check_budget` 在 key 无消费时抛 402 "查询消费失败")。聚合 REAL 列须外层 `CAST(... AS REAL)`(`check_budget`、`metrics/query` stats 查询均如此)

## 调试技巧

- 查看请求日志：`usage_logs` 表记录了每次请求的 input/output tokens、cost、latency、attempt stats
- 查看调度决策：`scheduler/trace.rs` 有 trace 日志，关注 `score` 和 `selection` 相关 tracing
- 测试单个模块：`cargo test --lib relay::pipeline` 跑子模块测试
- 健康检查：`GET /api/v1/health` 返回 `needs_setup` 判断是否需要初始化
