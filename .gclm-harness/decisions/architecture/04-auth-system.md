# 认证与初始化系统

## 设计目标

- 单用户模式（个人/小团队使用）
- 首次启动时初始化管理员密码
- Web 管理面板需要登录
- API 代理端点不需要登录（客户端用 API Key 认证）

## 初始化流程

```
首次启动
    │
    ▼
检测 database.users 表是否为空
    │
    ├─ 空 → 进入初始化模式
    │       ├─ Web 端：重定向到 /admin/setup
    │       └─ CLI 端：交互式输入密码
    │
    └─ 非空 → 正常启动
```

## 初始化页面

**路由**: `/admin/setup`（仅首次可用）

**流程**:
1. 输入管理员用户名（默认 `admin`）
2. 输入密码（最少 8 位）
3. 确认密码
4. 提交 → 创建用户 → 重定向到登录页

## 登录流程

**前端页面**: `/admin/login`（SPA 路由）

**认证方式**: JWT Token

```
POST /api/v1/admin/auth/login
Content-Type: application/json

{
  "username": "admin",
  "password": "xxx"
}

Response（统一管理 API 格式）:
{
  "code": 0,
  "message": "success",
  "data": {
    "token": "eyJ...",
    "expires_in": 86400
  }
}
```

**Token 存储**:
- 前端：localStorage
- 后端：JWT 签名验证（密钥存储在 TOML 配置文件）

## 权限模型

单用户模式，不需要 RBAC，只区分两种访问：

| 访问类型 | 认证方式 | 说明 |
|---------|---------|------|
| 管理面板 | JWT Token | Web 管理 API |
| 代理 API | API Key | 客户端调用（`/v1/*`） |

## 数据库 Schema

### users 表

```sql
CREATE TABLE users (
    id TEXT PRIMARY KEY,               -- UUID v7
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,       -- argon2id 哈希
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
```

## JWT 密钥配置

JWT 密钥存储在 TOML 配置文件中：

```toml
[auth]
jwt_secret = "your-secret-key-here"  # 首次运行时自动生成并写入 config.toml
token_expiry_hours = 24
```

**首次运行行为**:
1. 检测 `auth.jwt_secret` 是否存在
2. 如果不存在，随机生成并写入 config.toml
3. 日志提示用户保管好密钥

## API 端点

| 端点 | 方法 | 说明 | 认证 |
|------|------|------|------|
| `/api/v1/init` | POST | 初始化管理员（`InitRequest{username,password,site_title?}`） | 无（仅首次） |
| `/api/v1/admin/auth/login` | POST | 登录 | 无 |
| `/api/v1/admin/auth/me` | GET | 当前用户信息 | JWT |
| `/api/v1/admin/auth/password` | PUT | 修改密码 | JWT |

> **注**：当前实现无 logout 端点（JWT 自身无状态，前端清除 localStorage 即可）。未来若要主动失效 token 需引入 token 黑名单，建议按需追加。

## 中间件

```rust
// axum 中间件链
let admin_routes = Router::new()
    .nest("/api/v1/admin", admin_api_routes)
    .layer(axum::middleware::from_fn(auth_middleware));

// auth_middleware 逻辑：
// 1. 检查 Authorization: Bearer {token}
// 2. 验证 JWT 签名和过期时间
// 3. 失败返回 401
```

## 前端路由守卫

```tsx
// React Router 路由守卫
function ProtectedRoute({ children }) {
  const token = localStorage.getItem('token');

  if (!token) {
    return <Navigate to="/admin/login" />;
  }

  // 验证 token 有效性
  const { data: user } = useQuery('/api/v1/admin/auth/me');

  if (!user) {
    return <Navigate to="/admin/login" />;
  }

  return children;
}
```

## 安全考虑

| 考虑点 | 措施 |
|--------|------|
| 密码存储 | argon2id 哈希（Rust: `argon2` crate） |
| JWT 密钥 | 首次运行时随机生成，存储在 TOML 配置文件 |
| Token 过期 | 默认 24 小时，可配置 |
| 暴力破解 | **未实现**（单用户模式，文档早期规划） |
| HTTPS | 生产环境建议反向代理 + TLS |

## 配置扩展

当前实现中 `AppConfig.auth` 仅支持：

```toml
[auth]
jwt_secret = ""                # 首次运行自动生成
token_expiry_hours = 24        # JWT Token 过期时间
```

文档早期规划中的 `max_login_attempts` / `lockout_minutes` 暂未落地（单用户模式，登录保护优先级低）。

## CLI 命令（未实现）

文档早期版本规划的 `galaxy-router init` / `reset-password` 子命令当前**未实现**。当前 CLI 参数仅支持：

- `--config <path>`：配置文件路径（默认 `config.toml`）
- `--port <u16>`：覆盖 `[server] port`
- `--host <ip>`：覆盖 `[server] host`
- `--log-level <level>`：覆盖 `[logging] level`

如需 CLI 初始化/重置密码，请走 Web 端 `/api/v1/init` 或在数据库上手动更新 `users.password_hash`。
