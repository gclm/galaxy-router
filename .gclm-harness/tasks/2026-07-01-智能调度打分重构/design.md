---
doc_type: design
feature: 2026-07-01-智能调度打分重构
status: approved
complexity: complex
---

# 智能调度打分重构 设计方案

> scheduler/relay 候选打分层重构。解决「首选一家独大、新渠道被埋没、并发不分散、priority 形同虚设」四个失效点，让「速度优先 + 主动控制优先级 + 并发分散」真正生效。

## 1. 背景与根因

线上 brew 部署诊断发现 glm-5.2 请求 9507 次全走智谱，商汤（新建）0 次、京东（TPM 扛不住大上下文）已手动摘除。表面看是「商汤轮不到」，根因排查后定位到 **scheduler 打分层 4 个结构性失效**，不是重试/熔断 bug：

| # | 失效点 | 位置 | 表现 |
|---|---|---|---|
| ① | **priority 权重形同虚设** | scoring.rs:60 `1/(1+p)` | priority=1 vs 3 只差 0.25 分，被 latency/health 盖过。用户「优先用某渠道」使不上劲 |
| ② | **latency 打分完全失效** | candidates.rs:40-41 `min/max_latency_ms` 恒 None → scoring.rs:69 `(Some(_),_,_)=>1.0` | 任何有延迟数据的渠道都拿满分，**不反映快慢**。「速度优先」根本没实现 |
| ③ | **latency 冷启动埋没新渠道** | scoring.rs:70 `_ => 0.5` | 无历史数据的新渠道（商汤）永远比有数据的（智谱）低 0.3 分，上不了场 |
| ④ | **并发分散失效** | state.rs:70 `load_rate`（max_concurrency=0 时恒 0）+ candidates.rs:36-37 `waiting_count` 硬编码 0 | load 和 queue 双双失效，并发请求全部堆首选渠道 |

补充：`groups.max_retries` 是死字段（relay 代码不读），配置误导。

> 注：诊断基于当前工作区代码（已 commit `5438140` 代理可靠性增强）。#1-#4 是结构性问题，与该 commit 无关。

## 2. 目标与约束

**用户目标**：不执着首选是谁，但要「一直可用 + 速度不慢 + 能主动指定优先用某渠道（包月先用 / 测试新渠道）」。

**核心行为**：
1. priority 硬分档：高档优先用完（含换 key）才轮低档；档内按 latency/健康/并发打分选最快
2. 故障自动降档：当次请求中 health=0 的高档渠道降到下一档，不拖慢响应；恢复后自动回本档
3. latency 真实反映快慢：候选间相对归一化，快的打分高
4. 新渠道不被冷启动埋没：无历史数据给乐观默认分
5. 并发分散：基于实时在途并发数（最小连接数），不依赖 max_concurrency 配置

**成功标准（可验证）**：
- 商汤 priority=1、智谱=2 → 发请求首选商汤（即使智谱历史延迟更低）
- 商汤 health=0 时 → 当次降到智谱档，请求走智谱
- mock 两候选 latency 100ms / 1000ms → 100ms 的打分高（同 priority 档内）
- 新建无历史渠道 → 打分不被 0.5 埋没，有机会进 top_k
- 两候选 active 0 / 5 → active=0 的打分高
- max_retries 在前端不再可见；DB 迁移后 groups 无 max_retries 列

**明确不做**（可 grep/测试反向核对）：
- 不做按请求 token 大小路由
- 不改两层重试结构（`run_key_loop` 渠道内换 key 保留，run.rs 外层遍历保留）
- 不动 error 归因 / 失败日志逻辑（`5438140` 刚收口）
- 不动 per-key 熔断 / channel 黑名单 / sticky session
- 不做自适应限流（动态调整 max_concurrency）——LLM 瓶颈在上游 RPM/TPM，收益低、复杂度高
- `max_concurrency` 字段保留（默认 0=无限制，前端标注可选），不 DROP 该列

## 3. 方案

### 3.1 名词层（打分因子重新定义）

| 因子 | 现状 | 变化 |
|---|---|---|
| **priority** | 软因子 `1/(1+p)`，失效 | **硬分档**（不进打分公式，作为候选分组键） |
| **latency** | 有数据就满分（min/max None） | **候选间相对归一化**（填入集内 min/max），真实反映快慢 |
| **latency 冷启动** | 无数据 0.5 | 无数据 **0.8**（乐观，不埋没新渠道，也不反超已知最快） |
| **least_conn（新）** | 不存在 | **新增**：候选间按实时 active 相对归一化，active 少打分高 |
| **load** | max_concurrency=0 失效 | **移除**（被 least_conn 取代） |
| **queue** | waiting_count 硬编码 0 | **移除**（galaxy try_acquire 不排队，结构上无用） |
| **error_rate / health** | 有效 | **保留** |

### 3.2 编排层：candidates.rs 两阶段排序

**现状**（candidates.rs:88-127）：`find_group` → `group_items` 去重 → `score_candidates` 一次打分 → top_k=3。

**变化**：改成「分档 → 档内打分 → 拼接」：

```
group_items
  │
  ├─ ① 按 priority 分档（priority 值小=高档）
  │
  ├─ ② 每档内：score_candidates 打分（公式见 3.3，不再含 priority 因子）
  │     · 自动降档：档内候选若 health=0（is_available=false 或 health 分量=0），
  │       当次移到下一档末尾（不改 DB priority，只影响当次排序）
  │
  └─ ③ 档间拼接：高档全部（档内打分序）→ 低档全部 → 作为候选顺序交给 run.rs
```

外层 `run.rs::execute` 遍历逻辑不变（for candidate 全遍历 + capacity/熔断检查），只是候选顺序变了。

**自动降档判定**（避免歧义）：
- 触发条件：候选 `health == 0.0`（runtime.health()=0 或 is_available=false 导致 health 分量=0）
- 效果：当次请求该候选排到下一档末尾；下一个请求若 health 恢复则自动回本档（无状态，每次请求独立判定）

### 3.3 打分公式（scoring.rs）

```
score = w_latency  * latency_factor      // 候选间相对，[0,1]
      + w_least    * least_conn_factor   // 候选间相对，[0,1]，新增
      + w_error    * (1 - error_rate)    // 保留
      + w_health   * health              // 保留
```

- **移除**：`priority`（硬分档）、`load`、`queue` 三项
- **latency_factor**：`(latency, min, max)` 集内归一化 `1 - (latency-min)/(max-min)`；无数据 → 0.8
- **least_conn_factor**：`(active, min_active, max_active)` 集内归一化 `1 - (active-min)/(max-min)`；全 0 时 → 1.0（低负载不惩罚）
- 权重默认值（待 review 微调）：latency 1.0、least 1.0、error_rate 1.5、health 1.0

### 3.4 改动清单（挂载点）

| 文件 | 改动 |
|---|---|
| `scheduler/scoring.rs` | 移除 priority/load/queue 因子；加 least_conn_factor；latency 无数据 0.8；`CandidateScoreInput` 字段调整（加 active_concurrency，删 waiting_count/max_waiting_count/min/max 留用） |
| `relay/candidates.rs` | `score_candidates` 改两遍扫（先算集内 min/max latency + min/max active，再填入）；`build_relay_candidates` 改两阶段排序（分档 + 档内打分 + 自动降档） |
| `scheduler/state.rs` 或 `capacity.rs` | 暴露 per-channel active 读取 API（**假设：capacity 已跟踪 active，load_rate 内部已用，需确认/补公开方法**） |
| `api/handlers/admin/groups.rs` + 前端 | 前端隐藏 max_retries 配置项；max_concurrency 标注「可选/留空=无限制」；priority 配置加说明文案 |
| `src/db/migrations/{ver}_drop_unused_fields.sql` | 新增迁移：DROP `groups.max_retries`、`channels.custom_headers`、`channels.extras`（最后一步） |

### 3.5 数据库清理（最后一步）

受迁移约束 5 限制，只能**新增**迁移文件 DROP COLUMN（不改旧文件）。SQLite 3.35+ 支持 `ALTER TABLE ... DROP COLUMN`。

顺序硬约束：**代码全部改完（不再 SELECT/INSERT 这些列）→ cargo check 通过 → 才加 DROP 迁移**，否则编译失败。

DROP 列：`groups.max_retries`、`channels.custom_headers`、`channels.extras`。
**不 DROP**：`channels.max_concurrency`（保留默认 0）。

> ⚠️ 不可逆性：DROP 列后无法回滚（迁移约束）。卸载本 feature 只能代码层恢复，DB 列不回。

## 4. 验收契约（输入 → 期望输出）

| 输入 | 期望 |
|---|---|
| 候选 [商汤(p=1), 智谱(p=2)]，都健康 | 排序：商汤 > 智谱（高档在前） |
| 候选 [商汤(p=1, health=0), 智谱(p=2)] | 排序：智谱 > 商汤（自动降档） |
| 候选 [A(p=1,lat=100ms), B(p=1,lat=1000ms)] | A.score > B.score（latency 相对归一化生效） |
| 候选 [新渠道(无 latency), 智谱(有数据)] | 新渠道 latency_factor=0.8，不被 0.5 埋没 |
| 候选 [A(active=0), B(active=5)] | A.least_conn_factor > B（并发分散） |
| 候选 active 全 0 | least_conn_factor 全 1.0（低负载不惩罚） |
| 前端 group 编辑页 | 无 max_retries 字段；max_concurrency 标注可选 |
| 迁移后 `pragma_table_info('groups')` | 无 max_retries 列 |

## 5. 推进策略（分阶段）

| 阶段 | 内容 | 退出信号 |
|---|---|---|
| **P1 打分层** | scoring.rs 打分公式重构（latency 相对化 + 冷启动 0.8 + least_conn + 移除 priority/load/queue）；补全 scoring 单测覆盖验收契约 | `cargo test --lib scheduler::scoring` 全过 |
| **P2 候选层** | candidates.rs 两阶段排序（分档 + 档内打分 + 自动降档）；确认 capacity active 读取；补 candidates 单测 | `cargo test --lib relay::candidates` 全过 + `cargo check` |
| **P3 前端 + admin** | 前端隐藏 max_retries、标注 max_concurrency 可选、priority 文案；admin API 不变（字段保留） | `pnpm build` 通过 |
| **P4 DB 清理** | 新增 DROP 迁移（max_retries/custom_headers/extras） | `make test`（迁移在测试 DB 生效）+ `pragma_table_info` 验证 |

每阶段独立可编译、可测试。P1/P2 是核心，P3/P4 收尾。

## 6. 可卸载清单（边界自检）

拔掉这个 feature 要动的地方：
1. scoring.rs：恢复 load/queue/priority 因子，移除 least_conn_factor，latency 无数据改回 0.5
2. candidates.rs：恢复单次 score_candidates 排序，移除分档 + 自动降档
3. 前端：恢复 max_retries 配置项
4. **DB 迁移不可回滚**——DROP 的列（max_retries/custom_headers/extras）无法恢复。这是「废弃字段清理」的固有代价，design 已标注。

> 若担心不可逆，P4 可推迟（字段留着不影响功能，只是脏）。由用户在 review 时定。
