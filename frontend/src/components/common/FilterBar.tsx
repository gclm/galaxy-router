import { Search, RefreshCw } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Loader2 } from 'lucide-react'

interface FilterBarProps {
  searchValue: string
  onSearchChange: (v: string) => void
  searchPlaceholder?: string
  statusValue?: string
  onStatusChange?: (v: string) => void
  statusOptions?: { label: string; value: string }[]
  onRefresh: () => void
  loading?: boolean
  extra?: React.ReactNode
}

export function FilterBar({
  searchValue,
  onSearchChange,
  searchPlaceholder = '搜索...',
  statusValue,
  onStatusChange,
  statusOptions,
  onRefresh,
  loading,
  extra,
}: FilterBarProps) {
  return (
    <div className="flex flex-wrap items-center gap-3">
      <div className="relative flex-1 min-w-[200px] max-w-sm">
        <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
        <input
          type="text"
          value={searchValue}
          onChange={(e) => onSearchChange(e.target.value)}
          placeholder={searchPlaceholder}
          className="input pl-9"
        />
      </div>
      {statusOptions && onStatusChange && (
        <select
          value={statusValue ?? ''}
          onChange={(e) => onStatusChange(e.target.value)}
          className="input w-auto min-w-[120px]"
        >
          <option value="">全部状态</option>
          {statusOptions.map((opt) => (
            <option key={opt.value} value={opt.value}>{opt.label}</option>
          ))}
        </select>
      )}
      <Button
        variant="outline"
        size="icon"
        onClick={onRefresh}
        disabled={loading}
        title="刷新"
      >
        {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
      </Button>
      {extra}
    </div>
  )
}
