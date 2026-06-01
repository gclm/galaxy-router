# SOP: 编写排查 SOP

## 触发条件

修完一个 bug 后，如果满足以下任一条件，将排查经验写入或追加到对应 SOP：

- 排查过程超过 3 个诊断步骤
- 涉及生产数据库查询（`/opt/homebrew/var/lib/galaxy-router/galaxy.db`）
- 涉及 curl 上游验证 SSE 格式
- 根因是供应商兼容性问题而非代码逻辑错误

**不要提前写空模板** — 没有实际排查经验填充的 SOP 是噪音。

## 文件组织

```
.gclm-harness/sop/
├── _template.md           # 本文件：模板 + 写作规范
├── homebrew-deploy.md     # 部署运维
├── token-stats-debug.md   # token 统计缺失
└── <故障域>.md            # 按故障域命名，snake_case
```

故障域划分参考（不限于）：

| 故障域 | 典型问题 |
|--------|----------|
| `token-stats-debug` | token 为 0、usage 提取失败、cache_tokens 丢失 |
| `protocol-conversion` | 协议转换异常、流式转发中断、SSE 格式不兼容 |
| `channel-routing` | 渠道选择错误、负载均衡不均、重试未触发 |
| `db-migration` | 表结构变更、数据迁移失败 |

## 创建清单

新建 SOP 文件后，**必须**完成以下步骤：

- [ ] 文件放在 `.gclm-harness/sop/<故障域>.md`
- [ ] 行数 ≤ 100（超过就拆分）
- [ ] 诊断步骤中命令可直接复制执行
- [ ] 在 `AGENTS.md` 末尾 `## 排查 SOP` 节下追加一行链接
- [ ] 本地 `git add` 前确认以上全部完成

## SOP 模板

每个 SOP 文件使用以下三段式结构：

```markdown
# SOP: <故障域简述>

## 触发条件
— 什么时候该看这个 SOP（1-2 句）

## 诊断步骤
— 编号步骤，每步包含可直接复制的 SQL / curl / grep 命令
— 每步后面注明「→ 说明什么」的判断分支

## 修复清单
— 已知 root cause + 对应修复的表格
— 按日期倒序追加，最新的在最上面
```

## 写作规范

1. **命令可直接复制执行** — 用完整路径（`/opt/homebrew/...`），不要省略参数
2. **每步有判断分支** — 不要只列命令，要说明「如果看到 X → 说明是 Y 问题，跳到第 N 步」
3. **修复清单按日期追加** — 最新修复在最上面，不删旧记录
4. **引用代码位置** — 用 `文件:行号` 格式（如 `src/proxy/mod.rs:2280`）
5. **不超过 100 行** — 超过就该拆分成更细粒度的故障域

## AGENTS.md 关联

在 AGENTS.md 末尾 `## 排查 SOP` 节下，每个 SOP 一行链接：

```markdown
- [<简述>](.gclm-harness/sop/<文件名>.md) — 一句话说明覆盖什么
```

新增 SOP 时追加一行即可，不需要更复杂的索引结构。
