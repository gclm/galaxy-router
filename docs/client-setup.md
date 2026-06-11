# 客户端配置指南

本文档介绍如何将 Galaxy Router 配置到常见的 AI 编程工具中。

## 前提条件

1. Galaxy Router 已部署并运行（默认 `http://127.0.0.1:8080`）
2. 已在管理面板中创建 API Key（格式 `sk-gr-xxxx`）
3. 已创建对应的分组和渠道

## 通用参数

| 参数 | 值 |
|------|----|
| API Key | 在管理面板「API Keys」页面创建，格式 `sk-gr-xxxx` |
| OpenAI 兼容 Base URL | `http://<host>:8080/v1` |
| Anthropic 兼容 Base URL | `http://<host>:8080` |

---

## Codex CLI

OpenAI 官方命令行编程助手。

```bash
export OPENAI_API_KEY="sk-gr-xxxx"
export OPENAI_BASE_URL="http://127.0.0.1:8080/v1"
```

写入 shell 配置文件使其永久生效：

```bash
# Bash
echo 'export OPENAI_API_KEY="sk-gr-xxxx"' >> ~/.bashrc
echo 'export OPENAI_BASE_URL="http://127.0.0.1:8080/v1"' >> ~/.bashrc

# Zsh
echo 'export OPENAI_API_KEY="sk-gr-xxxx"' >> ~/.zshrc
echo 'export OPENAI_BASE_URL="http://127.0.0.1:8080/v1"' >> ~/.zshrc
```

---

## Claude Code

Anthropic 官方命令行编程助手。

```bash
export ANTHROPIC_API_KEY="sk-gr-xxxx"
export ANTHROPIC_BASE_URL="http://127.0.0.1:8080"
```

写入 shell 配置文件使其永久生效：

```bash
# Bash
echo 'export ANTHROPIC_API_KEY="sk-gr-xxxx"' >> ~/.bashrc
echo 'export ANTHROPIC_BASE_URL="http://127.0.0.1:8080"' >> ~/.bashrc

# Zsh
echo 'export ANTHROPIC_API_KEY="sk-gr-xxxx"' >> ~/.zshrc
echo 'export ANTHROPIC_BASE_URL="http://127.0.0.1:8080"' >> ~/.zshrc
```

> **注意：** `ANTHROPIC_BASE_URL` 不要带 `/v1` 后缀，Claude Code 会自动拼接 `/v1/messages`。

---

## Cursor

AI 代码编辑器，支持自定义 API 端点。

### 配置步骤

1. 打开 Cursor：`Settings` → `Models`
2. 在 **OpenAI API Key** 中填入 `sk-gr-xxxx`
3. 在 **OpenAI Base URL** 中填入 `http://127.0.0.1:8080/v1`
4. 点击 **Verify** 验证连通性
5. 在模型列表中选择你需要的模型

> **提示：** 如果使用 Claude 模型，同样通过 OpenAI 兼容端点接入（Galaxy Router 会自动做协议转换）。

---

## Cline

VS Code 中的 AI 编程助手插件。

### 配置步骤

1. 打开 Cline 侧边栏，点击设置图标 ⚙
2. **API Provider** 选择 `OpenAI Compatible`
3. **Base URL** 填入 `http://127.0.0.1:8080/v1`
4. **API Key** 填入 `sk-gr-xxxx`
5. **Model** 填入你在分组中配置的模型名称（如 `gpt-4o`、`claude-sonnet-4-20250514`）

> **提示：** Cline 也支持 `Anthropic` Provider，此时 Base URL 填 `http://127.0.0.1:8080`（不带 `/v1`），API Key 同样使用 `sk-gr-xxxx`。

---

## OpenClaw

开源命令行 AI 编程助手，兼容 Claude Code。

```bash
export ANTHROPIC_API_KEY="sk-gr-xxxx"
export ANTHROPIC_BASE_URL="http://127.0.0.1:8080"
```

配置方式与 Claude Code 相同。

---

## Hermes

AI 编程助手工具。

### 配置步骤

1. 打开 Hermes 设置
2. **API Provider** 选择 `OpenAI Compatible` 或 `Custom`
3. **Base URL** 填入 `http://127.0.0.1:8080/v1`
4. **API Key** 填入 `sk-gr-xxxx`
5. **Model** 选择你在 Galaxy Router 中配置的模型

---

## 快速配置脚本

项目提供了交互式配置脚本，自动生成对应工具的环境变量命令：

```bash
bash scripts/setup-client.sh
```

---

## 常见问题

### 连接失败

- 确认 Galaxy Router 正在运行：`curl http://127.0.0.1:8080/api/v1/health`
- 确认 Base URL 格式正确：OpenAI 兼容带 `/v1`，Anthropic 兼容不带 `/v1`
- 如果 Galaxy Router 部署在远程服务器，将 `127.0.0.1` 替换为实际地址

### 模型不存在

- 确认管理面板中已创建对应模型的分组
- 确认 API Key 有权访问该模型（检查「可用模型」配置）
- 模型名称需要与分组名称或正则匹配规则一致

### API Key 无效

- 确认使用的是 Galaxy Router 生成的 `sk-gr-xxxx` 格式 Key，而非上游服务商的 Key
- 确认 Key 未过期且处于启用状态
