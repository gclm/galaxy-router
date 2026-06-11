<div align="center">
  <img src="frontend/public/brand.svg" alt="Galaxy Router" width="120" />
  <h1>Galaxy Router</h1>
</div>

AI 协议互转代理网关，支持 OpenAI Chat Completions、OpenAI Responses、Anthropic Messages 三种协议互转。

## 功能特性

- **协议互转**: OpenAI Chat ↔ OpenAI Responses ↔ Anthropic Messages
- **多端点渠道**: 一个渠道支持多种协议端点
- **负载均衡**: 自适应加权评分 + 粘性会话
- **统计系统**: 按 Key/模型/渠道/时间维度统计用量和成本
- **Web 管理**: 渠道、分组、API Key 管理
- **操练场**: 内置多协议调试界面
- **模型定价**: 自动同步上游模型定价和能力数据

## 文档列表

| 文档 | 说明 |
|------|------|
| [installation.md](installation.md) | 安装指南（Homebrew/Docker/源码构建/GitHub Release） |
| [user-guide.md](user-guide.md) | 用户手册（核心概念、渠道/分组/API Key 管理、统计分析） |
| [client-setup.md](client-setup.md) | 客户端配置指南（Codex CLI / Claude Code / Cursor / Cline / OpenClaw / Hermes） |
