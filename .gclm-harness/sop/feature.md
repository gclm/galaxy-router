# 新功能开发 SOP

## 为什么需要这个 SOP

AI 直接拿到需求就写代码会出三个老问题：

1. **名字跟原代码对不上** — 没读架构就起名，概念冲突后 git blame 找十倍时间
2. **改着改着改出范围** — 没有"明确不做"，feature PR 变成一坨综合改动
3. **改完不留存档** — 三个月后没人知道当时为什么这样设计

本 SOP 在"需求"和"代码"之间塞了一份方案文件（design.md），让两边有交接点。

## 复杂度判断（先做这步）

| 复杂度 | 判断标准 | 走哪些 Gate |
|--------|----------|-------------|
| Trivial | 改配置/typo/单行改动 | 不走 SOP，直接改 |
| Simple | 1-2 文件，目标明确 | Gate 2-3 |
| Moderate | 多文件，有些模糊 | Gate 1-3 |
| Complex | 模糊/架构选择/多方案 | Gate 0-4 |

**执行**：AI 判断后告诉用户"我判为 {complexity}，走 Gate {gates}"，用户没反对就执行。

---

## Gate 0: 需求澄清（Complex 必须）

**前置**：无

**动作**：
1. 和用户对齐四件事：做什么 / 为谁 / 怎么算成功 / 不做什么
2. 能一句话说清 → 进入 Gate 1
3. 说不清 → 先 brainstorm（见 `sop/brainstorm.md`）

**退出**：需求摘要写入 task.yaml 的 notes 字段

**硬约束**：
- 不替用户决定"做什么"——碰到模糊角落停下来问，不自己挑一个
- "明确不做"必须具体到能被 grep 或测试反向核对，不写"不过度设计"这种空话

---

## Gate 1: 设计（Moderate+ 必须）

**前置**：Gate 0 完成（或 Simple 直接跳过）

**动作**：
1. 创建任务：`python3 .gclm-harness/tools/task.py create "{title}" feature {complexity}`
2. 读 `.gclm-harness/must/pitfalls.md`（避免已知坑）
3. 读 `.gclm-harness/architecture/` 相关子系统文档
4. 读 `.gclm-harness/reference/conventions.md`
5. 术语 grep 防冲突——新概念名没在代码/架构/历史 feature 里见过时，grep 一遍
6. 写 `tasks/{slug}/design.md`（模板见 `templates/task/design.md`）
7. frontmatter: `status: draft`
8. 人工 review → `status: approved`

**退出**：design.md `status: approved`

**硬约束**：
- AI 在 Gate 2 开始前必须运行 `python3 .gclm-harness/tools/task.py check-design {task-dir}`
- 不是 approved → 告诉用户"design 还没 approved，先 review"
- 不允许跳过

### 起草纪律

1. **别替用户做决定** — 用户没说清的角落写成"假设：……"，让用户能精确反驳
2. **目标和约束都写成可验证的** — 不写"让它能跑"，改写成"输入 A 时返回 B"
3. **每个 feature 都要能被卸载** — "如果想把它拔掉，要拔哪些地方？" 答不出说明边界没想清楚
4. **方案文件是给人概览的** — 每节超过一屏就砍或拆，示例优先于定义

### design.md 必须包含

- 第 1 节：决策与约束（用户目标/核心行为/成功标准/明确不做）
- 第 2 节：名词层 + 编排层（现状→变化两段式，含挂载点清单）
- 第 3 节：验收契约（输入→期望输出，可测试）
- 第 4 节：推进策略（分阶段实施，每阶段有退出信号）

---

## Gate 2: 实现（所有复杂度必须）

**前置**：
- Simple：无
- Moderate+：design.md `status: approved`（用 check-design 验证）

**动作**：
1. 按 design.md 第 4 节推进策略，生成 checklist.yaml
2. 严格按 steps 顺序执行
3. 每完成一步，更新 checklist.yaml status: pending → done
4. 不做方案外改动（记成"顺手发现"）
5. 发现设计缺失 → 停下来改 design.md（status 改回 draft），不硬冲

**退出**：所有 steps 完成，代码可运行

**硬约束**：
- 不允许引入 design.md 没定义的新概念（术语守护）
- 不允许加 `if (特殊情况)` 补丁分支，要加就改 design
- 不允许"顺手改"方案外的代码
- 不合并步骤——两步合做意味着出问题时不知道是哪一步引入的

### 实现姿态

1. **默认写最少的代码** — 只写当前步骤明确要的东西，不加"以后可能要"的可配置项
2. **只动该动的** — 同文件里别的函数风格丑、命名怪，除非和本次改动冲突，否则别碰
3. **design 没说的事别自己拍板** — 碰到没覆盖的角落，默认停下来回 design 谈

### 孤儿代码处理

你这次改动让某个 import/函数变成死代码 → 删掉。不是你改动造成的死代码 → 留着记成顺手发现。

---

## Gate 3: 验收（所有复杂度必须）

**前置**：Gate 2 完成

**动作**：
1. 逐条对照 design.md 第 3 节验收契约
2. lint + type-check + test 全绿
3. 检查 design.md 挂载点是否都实现
4. 写 report.md（验收报告）
5. 用户确认

**退出**：验收通过，用户确认

**验收报告**（report.md）格式：
```markdown
# 验收报告

## 验收契约核对
| 场景 | 期望 | 实际 | 通过 |
|------|------|------|------|

## 测试结果
- lint: ✓/✗
- type-check: ✓/✗
- test: ✓/✗ (N/M passed)

## 挂载点检查
- {file}: ✓/✗

## 顺手发现
- {file:line} {问题简述}。建议后续 {issue/refactor}。
```

---

## Gate 4: 收尾（Complex 必须）

**前置**：Gate 3 完成

**动作**：
1. 有教训？ → 写 `memory/reflections/{slug}.md`
2. 有决策？ → 写 `memory/decisions/{slug}.md`
3. architecture/ 或 reference/ 需要更新？ → 更新
4. 发现文档缺口？ → 更新 `memory/doc-gaps.md`
5. task 归档：`python3 .gclm-harness/tools/task.py finish {task-dir}`

**退出**：task.yaml status = completed

**反思触发条件**（不是每次都写）：
- 工作流失败（跳过了某个 Gate）
- 重复错误（同样的坑踩了两次）
- 缺失信号（design.md 没覆盖到某个场景）
- 持久的流程教训（下次应该默认这样做）

---

## 容易踩的坑

- 没读架构就动笔 — 方案跟现有代码对不上，术语冲突后 git blame 找十倍时间
- 用散文描述接口行为，没给具体示例 — 读者建不起模型
- 名词层/编排层只写"变化"不写"现状" — 读者无法判断变化是否合理
- 分批 review 方案 — 用户看不出全局一致性，整稿一次性给
- 在需求摘要里偷偷扩范围 — 验收时对不上
- 代码只写了一部分就发完成汇报 — 汇报只在全部完成后发一次
- 看到方案外的代码顺手改了 — 功能 PR 稀释成综合改动
- 关键场景清单一条都没落证据 — 测试通过 ≠ 验收场景满足
