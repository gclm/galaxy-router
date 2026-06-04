import { useState, useEffect, useCallback, useRef } from 'react'

interface UseAutoRefreshOptions {
  /** 刷新函数 */
  refetch: () => void
  /** 默认间隔(秒) */
  defaultInterval?: number
  /** localStorage 持久化 key */
  storageKey?: string
  /** 是否默认启用 */
  defaultEnabled?: boolean
}

export function useAutoRefresh({
  refetch,
  defaultInterval = 60,
  storageKey,
  defaultEnabled = false,
}: UseAutoRefreshOptions) {
  const [enabled, setEnabled] = useState(defaultEnabled)
  const [interval, setInterval_] = useState(() => {
    if (storageKey) {
      const stored = localStorage.getItem(storageKey)
      if (stored) return Number(stored) || defaultInterval
    }
    return defaultInterval
  })
  const timerRef = useRef<ReturnType<typeof globalThis.setTimeout> | null>(null)

  const setInterval = useCallback((seconds: number) => {
    const clamped = Math.max(5, Math.min(300, seconds))
    setInterval_(clamped)
    if (storageKey) localStorage.setItem(storageKey, String(clamped))
  }, [storageKey])

  const toggle = useCallback(() => setEnabled(v => !v), [])

  useEffect(() => {
    if (!enabled) {
      if (timerRef.current) clearTimeout(timerRef.current)
      return
    }

    // 页面不可见时暂停
    const handleVisibility = () => {
      if (document.hidden && timerRef.current) {
        clearTimeout(timerRef.current)
        timerRef.current = null
      } else if (!document.hidden) {
        // 恢复时立即刷新一次
        refetch()
      }
    }
    document.addEventListener('visibilitychange', handleVisibility)

    const tick = () => {
      if (document.hidden) return
      refetch()
      timerRef.current = setTimeout(tick, interval * 1000)
    }
    timerRef.current = setTimeout(tick, interval * 1000)

    return () => {
      document.removeEventListener('visibilitychange', handleVisibility)
      if (timerRef.current) clearTimeout(timerRef.current)
    }
  }, [enabled, interval, refetch])

  return { enabled, interval, setInterval, toggle, setEnabled } as const
}
