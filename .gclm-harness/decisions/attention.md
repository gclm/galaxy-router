# Attention

本文件是 CodeStable 技能启动必读的项目注意事项入口。所有 CodeStable 子技能开始工作前必须读取它。

## 项目碎片知识

<!-- cs-note managed: 用 cs-note 维护，新条目按下面分节追加 -->

### 编译与构建

- `make build` 构建项目（包含前端构建）
- `make frontend-build` 仅构建前端
- `make check` 代码检查
- `make watch` 监听自动构建
- 前端使用 pnpm，需要先安装依赖：`cd frontend && pnpm install`

### 运行与本地起服务

- `make run` 启动服务
- `make docker` 构建镜像，`make docker-run` 运行容器
- 配置文件: `config.toml`

### 测试

- `make test` 运行测试
- 单元测试: `#[test]` 或 `#[tokio::test]`
- 集成测试: `tests/integration_test.rs`
- 测试数据库: `/tmp/galaxy_test_*` 目录
- TDD 开发流程：先写测试，再写实现

### 命令与脚本陷阱

### 路径与目录约定

- `api/` HTTP 路由、请求处理
- `api/handlers/admin/` 管理 API 处理器
- `api/handlers/proxy/` 代理 API 处理器
- `api/middleware/` 认证中间件
- `auth/` 密码哈希、JWT
- `config.rs` TOML 配置加载
- `db/` 数据库连接、迁移

### 环境变量与凭证

- JWT 密钥: `config.toml` 中 `[auth] jwt_secret`，空值时首次运行自动生成
- 渠道 API Key 通过 Web 管理面板或 API 配置

### 其他

- ID 规范: 所有实体 ID 使用 UUID v7，TEXT 类型
- 数据库迁移: SQL 文件放在 `src/db/migrations/`，文件名 `{version}_{name}.sql`，version > 0；不可回滚、不可修改已发布版本（由 `sqlx::migrate!()` 编译期管理）
- 代理 API (`/v1/*`) 保持原生协议格式，不用统一响应包装
- 管理 API (`/api/v1/admin/*`) 使用统一 JSON 响应格式
- 渠道 `models` 字段结构: JSON 字符串数组，例 `["gpt-4o", "claude-3-5-sonnet"]`（`parse_models` 解析为 `Vec<String>`）
- 获取上游模型: `POST /api/v1/admin/fetch-models`（支持 OpenAI/Anthropic/Gemini）
