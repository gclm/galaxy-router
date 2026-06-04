import { Loader2 } from 'lucide-react'

interface EmptyStateProps {
  loading?: boolean
  isEmpty?: boolean
  loadingText?: string
  emptyText?: string
  colSpan?: number
  /** 是否为非表格场景 */
  standalone?: boolean
}

export function EmptyState({
  loading,
  isEmpty,
  loadingText = '加载中...',
  emptyText = '暂无数据',
  colSpan = 99,
  standalone = false,
}: EmptyStateProps) {
  if (loading) {
    const content = (
      <div className="flex items-center justify-center gap-2 py-8 text-sm text-muted-foreground">
        <Loader2 className="h-4 w-4 animate-spin" />
        {loadingText}
      </div>
    )
    return standalone ? content : <tr><td colSpan={colSpan}>{content}</td></tr>
  }

  if (isEmpty) {
    const content = (
      <div className="py-8 text-center text-sm text-muted-foreground">{emptyText}</div>
    )
    return standalone ? content : <tr><td colSpan={colSpan}>{content}</td></tr>
  }

  return null
}
