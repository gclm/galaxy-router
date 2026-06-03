# 运维层（Operations）

持续积累的文档，记录排查手册和问题复盘。

## 目录结构

```
operations/
├── sop/                # 排查手册（Standard Operating Procedure）
│   ├── _template.md    # SOP 模板
│   ├── homebrew-deploy.md
│   ├── token-stats-debug.md
│   └── ci-failure.md
└── issues/             # 问题记录
    └── _template.md    # 问题记录模板
```

## SOP 模板

```markdown
# {问题类型}排查手册

## 触发条件
什么情况下会遇到这个问题

## 诊断步骤
1. 检查 xxx
2. 查看 xxx 日志
3. 执行 xxx 命令

## 修复方案
- 情况A: 执行 xxx
- 情况B: 执行 xxx
```

## 问题记录模板

```markdown
# {问题简述}

## 现象
观察到什么异常

## 根因
问题的根本原因

## 解决方案
如何修复的

## 经验教训
如何避免再次发生
```
