# Project Overview

## Identity

Galaxy Router 是一个 AI 协议互转代理网关——客户端用 OpenAI/Anthropic/Responses 协议发请求，网关自动选择上游渠道并转换协议格式，支持负载均衡、自动重试、Key 轮换、预算控制。

## Boundaries

**做什么**：
- 接收 OpenAI Chat / OpenAI Responses / Anthropic Messages 三种入站协议
- 自动选择上游渠道（负载均衡 + 粘性会话 + 熔断 + Key 轮换）
- 协议互转（入站→出站格式转换 + SSE 流转换）
- 用量统计、成本估算、预算控制
- 多租户 API Key 管理（RPM/TPM 限流 + 分组权限）
- 前端管理面板（React SPA）

**明确不做什么**：
- 不做模型推理——纯代理转发
- 不做对话存储——只记录用量日志
- 不做多集群部署——单机 SQLite

## Major Areas

- **代理核心**: `relay/` — 请求生命周期：pipeline → candidates → executor → 上游转发
- **负载均衡**: `scheduler/` — 渠道评分、选择、熔断、容量管理、粘性会话
- **协议转换**: `protocol/` — inbound（入站解析）+ outbound（出站构建）+ SSE 流转换
- **API 层**: `api/` — handlers（admin CRUD + proxy 转发）+ middleware（认证/限流/CORS）
- **用量指标**: `metrics/` — token 计费估算、请求日志记录、统计查询
- **前端**: `frontend/` — React SPA，由 `static_assets.rs` 嵌入服务

## Technology

- Rust 2024 (axum 0.8, tokio 1.x)
- SQLite (sqlx 0.9)
- React + pnpm (前端 SPA)
