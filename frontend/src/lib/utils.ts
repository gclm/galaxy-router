import { clsx, type ClassValue } from "clsx"
import { twMerge } from "tailwind-merge"

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}

/**
 * 格式化后端返回的时间字符串。
 * 后端已在 SQL 查询中根据 timezone_offset 转换为本地时间，前端直接显示即可。
 */
export function formatDate(dateStr: string): string {
  const d = new Date(dateStr)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
}

export function formatNumber(n: number | undefined): string {
  return (n ?? 0).toLocaleString()
}

export function formatCost(n: number | null): string {
  return n != null ? `$${n.toFixed(6)}` : '-'
}

export function formatLatency(ms: number | null): string {
  if (ms == null) return '-'
  if (ms < 1000) return `${ms}ms`
  return `${(ms / 1000).toFixed(2)}s`
}

export function maskKey(key: string): string {
  if (key.length <= 12) return key
  return key.substring(0, 8) + '...' + key.substring(key.length - 4)
}

export function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`
  return n.toLocaleString()
}

/**
 * 复制文本到剪贴板。优先用 Clipboard API(需安全上下文 HTTPS/localhost),
 * 不可用时降级到 execCommand,保证 HTTP 部署也能复制。
 * 返回是否复制成功,不抛异常。
 */
export async function copyText(text: string): Promise<boolean> {
  if (navigator.clipboard) {
    try {
      await navigator.clipboard.writeText(text)
      return true
    } catch {
      // 安全上下文下仍可能被权限拒绝,降级到 execCommand
    }
  }
  const ta = document.createElement('textarea')
  ta.value = text
  ta.style.position = 'fixed'
  ta.style.top = '0'
  ta.style.opacity = '0'
  document.body.appendChild(ta)
  ta.focus()
  ta.select()
  let ok = false
  try {
    ok = document.execCommand('copy')
  } catch {
    // execCommand 失败时 ok 保持初始值 false
  }
  document.body.removeChild(ta)
  return ok
}
