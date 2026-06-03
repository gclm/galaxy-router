# 方案层（Solutions）

随迭代演进的文档，记录功能设计、优化方案和开发计划。

## 目录结构

```
solutions/
├── features/           # 功能方案
│   ├── models-api/     # 渠道模型 API
│   └── playground.md   # 操练场方案
├── optimizations/      # 优化方案
│   ├── architecture-optimization.md
│   └── frontend-redesign.md
└── roadmap.md          # 开发计划
```

## 文档模板

```markdown
# {标题}

## 目标
要解决什么问题

## 方案
技术设计、接口、流程

## 实施清单
- [ ] 步骤1
- [ ] 步骤2
- [ ] 步骤3

## 验收标准
怎么算完成
```

## 与 decisions/ 的区别

- `decisions/` = 已确定的事实，基本不变
- `solutions/` = 待实施的工作，包含设计和实施清单
