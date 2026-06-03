# Galaxy Router

AI 协议互转代理网关，支持 OpenAI Chat Completions、OpenAI Responses、Anthropic Messages 三种协议互转。

## 功能特性

- **协议互转**: OpenAI Chat ↔ OpenAI Responses ↔ Anthropic Messages
- **多端点渠道**: 一个渠道支持多种协议端点
- **负载均衡**: 自适应加权评分 + 粘性会话
- **统计系统**: 按 Key/模型/渠道/时间维度统计用量和成本
- **Web 管理**: 渠道、分组、API Key 管理
- **操练场**: 内置多协议调试界面
- **模型定价**: 自动同步上游模型定价和能力数据

## 文档

| 文档 | 说明 |
|---|---|
| [安装指南](docs/installation.md) | Homebrew、Docker、源码构建、二进制下载 |
| [使用手册](docs/user-guide.md) | 完整功能说明，含界面截图 |

## 快速开始

### Homebrew（macOS 推荐）

```bash
brew tap gclm/tap
brew install gclm/tap/galaxy-router
brew services start gclm/tap/galaxy-router
```

### Docker

```bash
docker pull ghcr.io/gclm/galaxy-router:latest
docker run -d -p 8080:8080 -v $(pwd)/data:/app/data ghcr.io/gclm/galaxy-router:latest
```

### 从源码构建

```bash
git clone https://github.com/gclm/galaxy-router.git
cd galaxy-router
make build
./target/debug/galaxy-router --config config.toml
```

访问 `http://127.0.0.1:8080` 进入管理界面，首次访问设置管理员密码。

## API 端点

### 代理 API

| 端点 | 方法 | 说明 |
|------|------|------|
| `/v1/chat/completions` | POST | OpenAI Chat |
| `/v1/responses` | POST | OpenAI Responses |
| `/v1/messages` | POST | Anthropic Messages |
| `/v1/embeddings` | POST | OpenAI Embedding |
| `/v1/images/generations` | POST | OpenAI Images |
| `/v1/models` | GET | 模型列表 |

### 客户端使用

任何 OpenAI / Anthropic SDK 只需将 `base_url` 指向 Galaxy Router：

```python
from openai import OpenAI

client = OpenAI(
    base_url="http://127.0.0.1:8080/v1",
    api_key="gp-your-api-key"
)
response = client.chat.completions.create(
    model="gpt-4o",
    messages=[{"role": "user", "content": "Hello"}]
)
```

## 开发

```bash
make dev          # 启动前后端开发环境
make test         # 运行测试
make check        # 格式化 + lint + 测试
make help         # 查看所有命令
```

## 许可证

[Apache License 2.0](LICENSE)
