import { BarChart3, Table2 } from 'lucide-react'
import { cn } from '@/lib/utils'

interface ViewToggleProps {
  showChart: boolean
  showTable: boolean
  onChartToggle: () => void
  onTableToggle: () => void
}

export function ViewToggle({ showChart, showTable, onChartToggle, onTableToggle }: ViewToggleProps) {
  const handleChart = () => {
    if (showChart && !showTable) return
    onChartToggle()
  }

  const handleTable = () => {
    if (!showChart && showTable) return
    onTableToggle()
  }

  const base = 'inline-flex items-center gap-1.5 rounded-lg border px-3 py-1.5 text-xs font-medium transition-all duration-150'

  return (
    <div className="inline-flex items-center gap-2">
      <button
        onClick={handleChart}
        className={cn(
          base,
          showChart
            ? 'border-primary/30 bg-primary/10 text-primary shadow-sm'
            : 'border-border bg-background text-muted-foreground hover:text-foreground hover:border-muted-foreground/30',
        )}
      >
        <BarChart3 className={cn('h-3.5 w-3.5', showChart && 'text-primary')} />
        图表
        {showChart && <span className="h-1 w-1 rounded-full bg-primary" />}
      </button>
      <button
        onClick={handleTable}
        className={cn(
          base,
          showTable
            ? 'border-primary/30 bg-primary/10 text-primary shadow-sm'
            : 'border-border bg-background text-muted-foreground hover:text-foreground hover:border-muted-foreground/30',
        )}
      >
        <Table2 className={cn('h-3.5 w-3.5', showTable && 'text-primary')} />
        表格
        {showTable && <span className="h-1 w-1 rounded-full bg-primary" />}
      </button>
    </div>
  )
}
