# Galaxy Router 架构总入口

> 状态：迁移完成
> 创建日期：2026-05-26

## 1. 项目简介

AI 协议互转代理网关，支持 OpenAI Chat Completions、OpenAI Responses、Anthropic Messages 三种协议互转。

## 2. 核心概念 / 术语表

- **渠道 (Channel)**: 一个上游服务提供商，支持多种协议端点
- **分组 (Group)**: 渠道分组，用于路由策略
- **API Key**: 客户端侧密钥，用于鉴权和统计
- **协议转换**: 请求/响应在不同 AI API 协议间互转
- **模型配置 (Models Config)**: 渠道的可用模型列表和模型映射配置
  - `available_models`: 从上游获取或手动添加的模型列表
  - `models`: 渠道支持的模型名列表（JSON 字符串数组，例 `["gpt-4o", "claude-3-5-sonnet"]`），模型映射通过 `group_items.model_name` 在分组层实现

## 3. 子系统 / 模块索引

| 模块 | 说明 | 详情 |
|------|------|------|
| api/ | HTTP 路由、请求处理 | [模块划分](02-module-design.md) |
| api/handlers/admin/fetch_models.rs | 模型获取服务 | 支持 OpenAI/Anthropic；含 Gemini 管理侧探测能力 |
| auth/ | 密码哈希、JWT | [认证系统](04-auth-system.md) |
| config.rs | TOML 配置加载 | [配置格式](03-config-format.md) |
| db/ | 数据库连接、迁移 | [配置格式](03-config-format.md) |
| frontend/ | 前端管理面板 | React + TypeScript + Vite |
| static_assets.rs | 静态文件服务 | 嵌入前端构建产物 |

### 详细架构文档

- [技术栈选型](01-tech-stack.md)
- [模块划分](02-module-design.md)
- [配置格式与数据库 Schema](03-config-format.md)
- [认证与初始化系统](04-auth-system.md)

## 4. 关键架构决定

见 [需求-已确认决策](../requirements/04-open-questions.md) 和 `docs/README.md` 审查修复记录。

## 5. 已知约束 / 硬边界

- 单用户模式（个人/小团队使用）
- SQLite 单机部署
- 正式代理路由仅支持 OpenAI Chat / OpenAI Responses / Anthropic Messages
