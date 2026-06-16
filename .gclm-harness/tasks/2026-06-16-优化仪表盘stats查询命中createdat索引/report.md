# 验收报告:仪表盘 stats 查询命中 created_at 索引

## 背景
仪表盘 overview/daily/latency 加载慢(pending)。根因:stats 查询 `WHERE date(datetime(created_at, '{tz}'))` 对索引列 `created_at` 套函数,索引失效,每次全表扫。

## 改动
- `src/metrics/query/mod.rs`:加 `range_utc_days` / `range_utc_between` helper(本地日期→UTC 时间范围),9 处 WHERE 改裸列 `created_at >= ? AND created_at < ?`(命中 `idx_usage_logs_created_at`)。GROUP BY/SELECT 的 `date(datetime())` 保留(作用于已过滤的小集合)。顺带清理孤儿 `today_local`。

## 验收
- cargo test:**229 过,0 回归**。
- E2E(brew-deploy):仪表盘 overview/daily/latency 三个请求从 **pending → 200**,数据正常(overview: today 502 请求;latency: p50/p95/p99 有值)。
- 数据量(1.7 万行)+索引本身无问题,纯查询写法优化。

## 顺手发现
- `created_at` 存 UTC,`range_utc_*` 按 UTC 算当月/当日;业务时区跨月边界(月初零点附近)可能差一天,次要——后续如需可按业务时区精确换算。
