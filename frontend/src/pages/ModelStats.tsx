import { useState, useMemo } from 'react'
import { useStatsModels } from '@/api/query-hooks'
import { PageHeader, FilterBar, EmptyState, SummaryCard, ViewToggle } from '@/components/common'
import { DistributionPieChart, CostBarChart } from '@/components/charts'
import { formatNumber, formatCost } from '@/lib/utils'
import { ArrowUpDown } from 'lucide-react'

export function ModelStats() {
  const [showChart, setShowChart] = useState(true)
  const [showTable, setShowTable] = useState(false)
  const [search, setSearch] = useState('')
  const [days, setDays] = useState(7)
  const [sortBy, setSortBy] = useState<'request_count' | 'total_cost' | 'input_tokens' | 'output_tokens'>('request_count')
  const [sortOrder, setSortOrder] = useState<'desc' | 'asc'>('desc')

  const { data, isLoading } = useStatsModels({ days })
  const stats = data ?? []

  // 图表始终使用全量数据
  const chartData = useMemo(() =>
    stats.map((s) => ({ name: s.model, value: s.request_count, cost: s.total_cost }))
  , [stats])

  // 表格过滤仅在 showTable 时生效
  const filtered = useMemo(() => {
    if (!showTable || !search) return stats
    return stats.filter((s) => s.model.toLowerCase().includes(search.toLowerCase()))
  }, [stats, search, showTable])

  const sorted = useMemo(() => {
    const items = [...filtered]
    items.sort((a, b) => {
      const av = a[sortBy]
      const bv = b[sortBy]
      return sortOrder === 'desc' ? (av > bv ? -1 : 1) : (av < bv ? -1 : 1)
    })
    return items
  }, [filtered, sortBy, sortOrder])

  const totals = stats.reduce(
    (acc, s) => ({
      requests: acc.requests + s.request_count,
      input: acc.input + s.input_tokens,
      output: acc.output + s.output_tokens,
      cost: acc.cost + s.total_cost,
    }),
    { requests: 0, input: 0, output: 0, cost: 0 },
  )

  const handleSort = (col: typeof sortBy) => {
    if (sortBy === col) {
      setSortOrder((o) => (o === 'desc' ? 'asc' : 'desc'))
    } else {
      setSortBy(col)
      setSortOrder('desc')
    }
  }

  const SortButton = ({ col, label }: { col: typeof sortBy; label: string }) => (
    <button
      className="inline-flex items-center gap-1 hover:text-foreground"
      onClick={() => handleSort(col)}
    >
      {label}
      {sortBy === col && <ArrowUpDown className="h-3 w-3" />}
    </button>
  )

  return (
    <div className="space-y-4">
      <PageHeader
        title="模型统计"
        subtitle="查看每个模型的请求量、Token 消耗和成本"
      />

      {/* 汇总卡片 */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
        <SummaryCard label="总请求" value={formatNumber(totals.requests)} />
        <SummaryCard label="总 Token" value={formatNumber(totals.input + totals.output)} />
        <SummaryCard label="输入 Token" value={formatNumber(totals.input)} />
        <SummaryCard label="总成本" value={totals.cost > 0 ? `$${totals.cost.toFixed(4)}` : '-'} />
      </div>

      {/* 工具栏 */}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <ViewToggle
          showChart={showChart}
          showTable={showTable}
          onChartToggle={() => setShowChart((v) => !v)}
          onTableToggle={() => setShowTable((v) => !v)}
        />
        <FilterBar
          searchValue={search}
          onSearchChange={setSearch}
          searchPlaceholder="搜索模型名称..."
          onRefresh={() => {}}
          loading={isLoading}
          extra={
            <select
              value={days}
              onChange={(e) => setDays(Number(e.target.value))}
              className="input w-auto min-w-[120px]"
            >
              <option value={1}>最近 1 天</option>
              <option value={7}>最近 7 天</option>
              <option value={30}>最近 30 天</option>
              <option value={90}>最近 90 天</option>
            </select>
          }
        />
      </div>

      {/* 图表区域 */}
      {showChart && (
        <div className="grid gap-5 md:grid-cols-2">
          <div className="rounded-2xl border bg-card p-5">
            <DistributionPieChart data={chartData} loading={isLoading} />
          </div>
          <div className="rounded-2xl border bg-card p-5">
            <CostBarChart data={chartData} title="模型成本 Top 8" loading={isLoading} />
          </div>
        </div>
      )}

      {/* 表格区域 */}
      {showTable && (
        <div className="rounded-2xl border bg-card overflow-hidden">
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b bg-muted/50">
                  <th className="text-left px-4 py-3 font-medium">模型</th>
                  <th className="text-right px-4 py-3 font-medium"><SortButton col="request_count" label="请求量" /></th>
                  <th className="text-right px-4 py-3 font-medium"><SortButton col="input_tokens" label="输入 Token" /></th>
                  <th className="text-right px-4 py-3 font-medium"><SortButton col="output_tokens" label="输出 Token" /></th>
                  <th className="text-right px-4 py-3 font-medium"><SortButton col="total_cost" label="成本" /></th>
                </tr>
              </thead>
              <tbody>
                <EmptyState
                  loading={isLoading}
                  isEmpty={!isLoading && sorted.length === 0}
                  loadingText="加载中..."
                  emptyText={search ? '没有匹配的模型' : '暂无统计数据'}
                  colSpan={5}
                />
                {!isLoading && sorted.map((row) => (
                  <tr
                    key={row.model}
                    className="border-b last:border-0 hover:bg-muted/30 transition-colors"
                  >
                    <td className="px-4 py-3 font-medium font-mono text-xs">{row.model}</td>
                    <td className="px-4 py-3 text-right tabular-nums">{formatNumber(row.request_count)}</td>
                    <td className="px-4 py-3 text-right tabular-nums text-muted-foreground">{formatNumber(row.input_tokens)}</td>
                    <td className="px-4 py-3 text-right tabular-nums text-muted-foreground">{formatNumber(row.output_tokens)}</td>
                    <td className="px-4 py-3 text-right tabular-nums">{formatCost(row.total_cost)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}
    </div>
  )
}
