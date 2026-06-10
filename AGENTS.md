# AGENTS.md

> AI 协议互转代理网关 · 内部协作入口
>
> 这是一张地图，不是说明书。详细规范在各专项文档中。

---

## 约束（不能违反）

这些是硬性规则，违反会导致架构漂移或系统不可用：

| # | 约束 | 原因 |
|---|------|------|
| 1 | 所有实体 ID 使用 UUID v7，TEXT 类型 | 全局唯一 + 时序可排序 |
| 2 | 管理 API (`/api/v1/admin/*`) 使用统一 JSON 格式：`{code, message, data}` | 与代理 API 分离，客户端 SDK 不受影响 |
| 3 | 代理 API (`/v1/*`) **保持原生协议格式**，不用统一响应包装 | 透传上游响应，确保 SDK 兼容 |
| 4 | 渠道 `models` 字段固定为 JSON 字符串数组：`["gpt-4o", "claude-3-5-sonnet"]` | 多处代码依赖此结构（`parse_models` / `Channel.models: Vec<String>`） |
| 5 | 数据库迁移：SQL 文件放在 `src/db/migrations/`，文件名 `{version}_{name}.sql` 且 version > 0；不可回滚、不可修改已发布版本 | 由 `sqlx::migrate!()` 在编译期管理，版本号即文件名 |
| 6 | 业务配置（channels / groups / api_keys / model_info）变更后，必须同步失效对应 `ProxyCache`（`invalidate_channel` / `invalidate_all_channels`） | 缓存与数据库一致性 |

**踩坑警告**：

- 不要把错误处理统一到 `error.rs`（不存在），错误类型在各模块内定义
- `protocol/inbound/` 和 `protocol/outbound/` 是**空目录**待重构，不要往里放文件
- `relay/` 为代理请求生命周期模块（合并原 `proxy/` 核心），`scheduler/` 为负载均衡模块

---

## 文档地图

按需深入，不要一次性读全部。

```
.gclm-harness/
├── decisions/           # 决策层 — "为什么这样设计"
│   ├── architecture/    # 架构决策（模块划分、配置、认证）
│   ├── requirements/    # 需求愿景（做什么 / 不做什么）
│   ├── attention.md     # 项目注意事项
│   └── conventions.md   # 文档体系规范
│
├── solutions/           # 方案层 — "怎么实现"
│   ├── features/        # 功能方案（含实施清单）
│   ├── optimizations/   # 优化方案
│   └── roadmap.md       # 开发计划
│
├── operations/          # 运维层 — "出了问题怎么办"
│   ├── sop/             # 排查手册
│   └── issues/          # 问题复盘
│
└── reference/           # 参考层 — 静态资料
    ├── analyses/        # 竞品分析
    └── conventions/     # 格式规范
```

| 我要... | 去哪里 |
|---------|--------|
| 了解架构为什么这样设计 | `decisions/architecture/` |
| 知道项目边界和不做的事 | `decisions/requirements/` |
| 实现一个新功能 | `solutions/features/` 找相似方案 |
| 排查问题 | `operations/sop/` |
| 查格式规范（备份、ID 等） | `reference/conventions/` |
| 写新文档 | `decisions/conventions.md` |

---

## 快速参考

| 项 | 值 |
|----|----|
| 语言 | Rust 2024 |
| Web 框架 | axum 0.8 |
| 数据库 | SQLite (sqlx 0.9) |
| 异步运行时 | tokio 1.x |
| 前端 | React + pnpm |
| 测试数据库 | `/tmp/galaxy_test_*` |

| 命令 | 作用 |
|------|------|
| `make build` | 构建项目（含前端） |
| `make run` | 启动服务 |
| `make test` | 运行测试 |
| `make check` | 代码检查 |
| `make watch` | 监听自动构建 |

---

## 写文档时

1. 先判断是 **决策** 还是 **方案**（决策=不变的事实，方案=待实施的工作）
2. 用 `decisions/conventions.md` 中的模板
3. 存放到对应目录，更新 README 索引
4. AGENTS.md **不重复**任何文档内容，需要时用链接引用

---

## Git 提交

```
<type>: <description>
```

`feat` / `fix` / `refactor` / `test` / `docs` / `chore`
