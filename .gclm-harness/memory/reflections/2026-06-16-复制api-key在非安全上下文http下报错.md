---
doc_type: reflection
date: 2026-06-16
feature: 2026-06-16-复制api-key在非安全上下文http下报错
tags: [frontend, clipboard, secure-context]
---

# 复制 API Key 在非安全上下文下报错 反思

## Expected vs Actual

- Expected: HTTP 部署下点击「复制」也能复制成功
- Actual: `navigator.clipboard` 在非安全上下文为 undefined,直接 `.writeText` 抛 TypeError

## Root Cause

- 直接调 `navigator.clipboard.writeText` 而未判断可用性、未降级。Web Clipboard API 受 Secure Context 限制,HTTP 下不存在该对象。

## Why Not Caught Earlier

- 本地开发走 localhost(安全上下文),永远不复现;只有部署到 HTTP 云端才暴露
- 前端无测试框架,也没有针对「复制」的逻辑测试能覆盖 clipboard 不可用的降级分支

## Next Time

- 任何用到 `navigator.clipboard` 的地方,统一走 `lib/utils.ts::copyText`(已建),不要再裸调
- 前端新增浏览器受限 API 调用时,先确认其 Secure Context / 权限要求,并写好降级路径
- 云端若上 HTTPS,原生 clipboard 即可用,降级仅作兜底(运维侧)
