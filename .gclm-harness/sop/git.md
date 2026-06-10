# Git 工作流 SOP

## 适用场景

所有涉及代码变更的任务完成后，统一走本 SOP 提交。

---

## 1. Commit Message 格式

采用 Conventional Commits：

```
<type>(<scope>): <description>

[optional body]

[optional footer]
```

### Type

| type | 用途 |
|---|---|
| `feat` | 新功能 |
| `fix` | 修 bug |
| `refactor` | 重构（不改变外部行为） |
| `test` | 加测试 / 改测试 |
| `docs` | 改文档 |
| `chore` | 构建/工具/依赖 |
| `style` | 格式化（不影响逻辑） |

### Scope（可选）

影响范围：`api` / `db` / `relay` / `ui` / `config` 等，根据项目模块定。

### Description

- 一句话，不超过 50 字符
- 用英文（项目约定）
- 不以大写开头，不加句号

---

## 2. 提交时机

| 任务类型 | 何时提交 |
|---|---|
| feature | Gate 收尾（验收通过后） |
| issue | Gate 收尾（修复验证后） |
| refactor | Gate 收尾（重构完成且测试通过后） |
| 多个小改动 | 每个逻辑单元一次提交，不攒一批 |

---

## 3. Scoped Commit（范围纪律）

**只提交本次任务相关的变更**：

- ✅ 本次改的代码 + 测试 + 文档
- ✅ 本次更新的知识层文件（architecture/ must/ 等）
- ❌ 顺手改的无关代码
- ❌ 不属于本次任务的"顺便优化"
- ❌ IDE 配置 / 个人偏好文件

**发现无关改动**：
- 已暂存 → `git reset` 撤出
- 未暂存 → 不管，留给后续任务

---

## 4. 提交前检查

```bash
# 1. 看改了什么
git status
git diff --stat

# 2. 跑测试
make test  # 或项目对应的命令

# 3. 跑 lint
make check  # 或 cargo clippy / npm run lint

# 4. 确认无遗漏
git diff  # 逐文件看
```

**任一检查失败 → 不提交，先修。**

---

## 5. 分支策略（可选，项目自定）

默认用主干开发（trunk-based）：
- `main` 分支始终可部署
- 功能分支命名：`feat/{slug}` / `fix/{slug}` / `refactor/{slug}`
- 合并前跑 CI，合并后用 squash 或 rebase（项目约定）

如果项目有自己的分支策略，以项目为准。

---

## 6. 禁止事项

- ❌ 提交未经测试的代码
- ❌ 提交包含调试代码（print / console.log / dbg!）
- ❌ 提交包含密钥 / 凭证
- ❌ 提交超大变更（>500 行）而不拆分
- ❌ 在 commit message 里写"fix bug" / "update code" 这种无意义描述

---

## 7. 与其他 SOP 的衔接

| 上游 SOP | 衔接点 |
|---|---|
| feature.md | Gate 4（收尾）→ 走本 SOP 提交 |
| issue.md | Gate 3（收尾）→ 走本 SOP 提交 |
| refactor.md | Gate 3（收尾）→ 走本 SOP 提交 |

收尾 Gate 会提示"走 git SOP 提交"，按本文件执行。

---

## 8. 顺手发现的记录

实现过程中发现的不在本次范围的改进点，**不提交**，记录到 task 的 report.md：

```markdown
## 顺手发现

- {file:line} {问题简述}。不在本次范围，建议后续 {issue/refactor}。
```

留给后续任务处理。
