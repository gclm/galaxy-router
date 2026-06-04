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
