# Octopus 项目分析

## 项目定位

简洁优雅的 LLM API 聚合与负载均衡服务，面向个人用户。

## 技术栈

- 后端: Go 1.25, Gin, GORM, Cobra, Viper, Zap
- 前端: Next.js, Tailwind CSS
- 数据库: SQLite / MySQL / PostgreSQL

## 我们借鉴什么

### 1. 分组配置设计

```go
type Group struct {
    Name              string      // 分组名称 = 对外暴露的 model 名
    Mode              GroupMode   // 负载均衡模式
    MatchRegex        string      // 匹配正则
    FirstTokenTimeOut int         // 首 Token 超时(秒)
    RetryEnabled      bool        // 同通道重试
    MaxRetries        int         // 最大重试次数
    Items             []GroupItem
}

type GroupItem struct {
    ChannelID int
    ModelName string // 实际使用的模型名
    Priority  int    // 优先级(Failover 模式用)
    Weight    int    // 权重(Weighted 模式用)
}
```

**借鉴点**: 简单直观，分组名即模型名，配置清晰。

### 2. Transformer 架构

```
入站协议 → 统一内部模型 → 出站协议
  (inbound)     (model)     (outbound)
```

**借鉴点**: 双向转换架构，inbound/outbound 分离。

### 3. 同格式直通优化

当入站和出站协议相同时，原始字节直接转发，不做解析。

**借鉴点**: 减少不必要的序列化开销。

## 已知问题（我们避免）

| 问题 | 说明 |
|------|------|
| Volcengine 空切片 panic | `idx = -1` 直接崩溃 |
| FinishReasonRefusal 映射错误 | 映射为 `failed` 而非 `completed` |
| Annotations 信息丢失 | round-trip 后 annotations 消失 |
| OutputIndex 类型不一致 | Inbound 用 `*int`，Outbound 用 `int` |

## 关键代码路径

| 模块 | 路径 |
|------|------|
| 分组模型 | `internal/model/group.go` |
| 负载均衡 | `internal/relay/balancer/balancer.go` |
| 入站转换 | `internal/transformer/inbound/` |
| 出站转换 | `internal/transformer/outbound/` |
| 统一模型 | `internal/transformer/model/model.go` |
