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

  return (
    <div className="inline-flex rounded-lg border bg-background p-0.5">
      <button
        onClick={handleChart}
        className={cn(
          'inline-flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs font-medium transition-colors',
          showChart
            ? 'bg-primary text-primary-foreground shadow-sm'
            : 'text-muted-foreground hover:text-foreground',
        )}
      >
        <BarChart3 className="h-3.5 w-3.5" />
        图表
      </button>
      <button
        onClick={handleTable}
        className={cn(
          'inline-flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs font-medium transition-colors',
          showTable
            ? 'bg-primary text-primary-foreground shadow-sm'
            : 'text-muted-foreground hover:text-foreground',
        )}
      >
        <Table2 className="h-3.5 w-3.5" />
        表格
      </button>
    </div>
  )
}
