# 冷启动 SOP

## 适用场景

项目已在开发，需要引入 Keel 工作流。

**核心原则**：
- 不追求完美：先有 70% 的内容，跑起来再迭代
- 不阻塞开发：如果某个文档缺失，先留 TODO，继续开发
- 每次任务后补一点：做完一个 feature，顺手补知识层

---

## Phase 1: 最小可用（30 分钟）

**目标**：让工作流能跑起来

### 1.1 创建目录结构

```bash
mkdir -p .gclm-harness/{must,overview,architecture,reference,guides,memory/{reflections,decisions},sop,tasks,tmp/investigations,tools}
```

### 1.2 复制文件

从 keel 项目复制：

```bash
# 工具脚本
cp {keel-project}/tools/*.py .gclm-harness/tools/

# SOP 文件
cp {keel-project}/templates/sop/*.md .gclm-harness/sop/

# 启动文件
cp {keel-project}/templates/startup.md .gclm-harness/startup.md
cp {keel-project}/templates/index.md .gclm-harness/index.md

# must 模板
cp {keel-project}/templates/must/*.md .gclm-harness/must/

# overview 模板
cp {keel-project}/templates/overview/project-overview.md .gclm-harness/overview/

# skill
mkdir -p .claude/skills/keel
cp {keel-project}/skills/keel/SKILL.md .claude/skills/keel/
```

### 1.3 创建空文件

```bash
touch .gclm-harness/architecture/ARCHITECTURE.md
touch .gclm-harness/reference/conventions.md
touch .gclm-harness/reference/repo-surfaces.md
touch .gclm-harness/memory/doc-gaps.md
echo "tmp/" > .gclm-harness/.gitignore
```

**此时可以开始用 Keel 做新任务**，但知识层是空的。

---

## Phase 2: AI 生成骨架（1 小时）

**目标**：从现有代码生成知识层

### 2.1 must/project-basics.md

从代码提取：
- 技术栈（Cargo.toml / package.json / go.mod）
- 启动命令（Makefile / scripts/）
- 关键约束
- 一屏读完

### 2.2 must/pitfalls.md

从 git log 提取最近 bug，总结常见坑：

```bash
git log --oneline --grep="fix" | head -10
```

### 2.3 overview/project-overview.md

项目身份、边界、主要领域、技术选型。

### 2.4 architecture/ARCHITECTURE.md

从 src/ 目录画模块划分：

- 模块职责（从 pub fn / pub struct 提取）
- 依赖关系（从 use 语句提取）
- 不变量
- 数据流

### 2.5 reference/conventions.md

从代码提取：
- 命名约定（读 5 个文件）
- 错误处理规范（读错误类型定义）
- 测试规范（读 tests/）

### 2.6 reference/repo-surfaces.md

从代码提取公开接口、命令、配置文件。

---

## Phase 3: 人 review（1-2 小时）

**目标**：确认 AI 生成的内容

### 3.1 overview/project-overview.md

- [ ] 项目身份对不对？
- [ ] 边界对不对？
- [ ] 主要领域对不对？

### 3.2 architecture/ARCHITECTURE.md

- [ ] 模块划分对不对？
- [ ] 数据流方向对不对？
- [ ] 不变量有没有漏的？

### 3.3 reference/conventions.md

- [ ] 命名约定对不对？
- [ ] 错误处理规范对不对？

### 3.4 must/pitfalls.md

- [ ] 硬约束有没有漏的？
- [ ] 常见坑有没有漏的？

### 3.5 SOP

- [ ] Gate 是否合理？
- [ ] 复杂度分级是否合理？

---

## Phase 4: 用第一个任务验证（半天）

**目标**：验证流程能跑通

### 4.1 选任务

选一个 Simple 复杂度的小功能：
- 加一个配置项
- 改一个接口的返回格式
- 加一个日志字段

### 4.2 按 SOP 走一遍

- 复杂度判断：Simple
- 走 Gate 2-3（实现 + 验收）
- 记录哪里卡住了

### 4.3 调整 SOP

- 哪个 Gate 多余？ → 删掉
- 哪个 Gate 不够？ → 加上
- 哪个步骤不清楚？ → 改清楚

### 4.4 记录到 CHANGELOG.md

```markdown
## {date}
- 初始版本
- Phase 4 验证发现：{发现}
```

---

## Phase 5: 逐步补全（持续）

**目标**：边用边补，不追求一次补全

每次做完一个任务，顺手补知识层：
- 做完 feature → 补 architecture 相关部分
- 踩到坑 → 补 must/pitfalls
- 做了决策 → 补 memory/decisions

---

## 关键原则

1. **不阻塞开发**：Phase 1 完成就可以开始用 Keel
2. **不追求完美**：70% 的内容就够了
3. **边用边补**：每次任务后补一点
4. **记录调整**：所有调整记录到 CHANGELOG.md
