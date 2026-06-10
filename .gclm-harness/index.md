# Index

## Categories

- `must/`: 每次启动必读的简短上下文
- `overview/`: 项目身份、边界和主要领域
- `architecture/`: 模块划分、数据流、所有权边界
- `reference/`: 稳定的查找事实（编码规范、仓库表面）
- `memory/`: 反思、决策和已知文档缺口
- `sop/`: 工作流流程定义（feature/refactor/issue/brainstorm/cold-start）
- `guides/`: 特定工作流的操作指南

## Key Documents

- `startup.md`: 启动阅读顺序
- `overview/project-overview.md`: 项目是什么、边界在哪
- `architecture/ARCHITECTURE.md`: 模块职责、依赖、不变量
- `reference/conventions.md`: 编码规范
- `reference/repo-surfaces.md`: 公开接口、命令、配置文件

## Routing Rules

- 正常开发：先读 `startup.md`
- 改架构前：先读 `architecture/ARCHITECTURE.md`
- 改公开接口前：先读 `reference/repo-surfaces.md`
- 改代理/协议逻辑前：先读 `overview/project-overview.md` 确认边界

## Memory

- `memory/reflections/`: 任务教训
- `memory/decisions/`: 技术决策
- `memory/doc-gaps.md`: 已知文档缺口
