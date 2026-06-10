---
doc_type: feature-design
status: draft
feature: 2026-06-10-cold-start
summary: 运行 cold-start SOP，从代码生成知识层骨架
---

# 冷启动 — 生成项目知识层

## 1. 决策与约束

### 用户目标
- 让 AI 从现有代码自动生成 .gclm-harness/ 下的知识层文件

### 成功标准
- must/project-basics.md 有内容（技术栈、核心模块、启动命令）
- must/pitfalls.md 有内容（至少 3 条硬约束/常见坑）
- architecture/ARCHITECTURE.md 有内容（模块划分、数据流）

### 明确不做
- 不追求完美，先生成 70% 正确的内容

## 2. 推进策略

参考 sop/cold-start.md 的 Phase 1 逐步执行。
