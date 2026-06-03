# 决策层（Decisions）

确定后基本不变的文档，记录项目的核心决策和约束。

## 目录结构

```
decisions/
├── architecture/       # 架构决策
│   ├── ARCHITECTURE.md # 架构总入口
│   ├── tech-stack.md   # 技术栈选型
│   ├── module-design.md # 模块设计
│   ├── config-format.md # 配置格式
│   └── auth-system.md  # 认证系统
├── requirements/       # 需求愿景
│   ├── project-vision.md    # 项目愿景
│   ├── core-requirements.md # 核心需求
│   ├── protocol-matrix.md   # 协议矩阵
│   └── open-questions.md    # 待确认问题
└── attention.md        # 项目注意事项
```

## 文档模板

```markdown
# {标题}

## 背景
为什么需要做这个决策

## 决策
我们选择了什么

## 理由
为什么这样选

## 约束
这个决策带来的限制
```
