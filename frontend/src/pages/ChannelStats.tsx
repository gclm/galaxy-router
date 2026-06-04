import { useState, useMemo } from 'react'
import { useStatsChannels } from '@/api/query-hooks'
import { PageHeader, FilterBar, EmptyState, SummaryCard, ViewToggle } from '@/components/common'
import { DistributionPieChart, ChannelCompareChart } from '@/components/charts'
import { formatNumber, formatCost } from '@/lib/utils'
import { ArrowUpDown } from 'lucide-react'

type SortField = 'request_count' | 'total_cost' | 'success_rate' | 'input_tokens' | 'output_tokens'

export function ChannelStats() {
  const [showChart, setShowChart] = useState(true)
  const [showTable, setShowTable] = useState(false)
  const [search, setSearch] = useState('')
  const [days, setDays] = useState(7)
  const [sortBy, setSortBy] = useState<SortField>('request_count')
  const [sortOrder, setSortOrder] = useState<'desc' | 'asc'>('desc')

  const { data, isLoading } = useStatsChannels({ days })
  const stats = data ?? []

  const pieData = useMemo(() =>
    stats.map((s) => ({ name: s.channel_name, value: s.request_count, cost: s.total_cost }))
  , [stats])

  const compareData = useMemo(() =>
    stats.map((s) => ({ name: s.channel_name, success: s.success_count, failure: s.failure_count }))
  , [stats])

  const filtered = useMemo(() => {
    if (!showTable || !search) return stats
    return stats.filter((s) => s.channel_name.toLowerCase().includes(search.toLowerCase()))
  }, [stats, search, showTable])

  const sorted = useMemo(() => {
    const items = [...filtered]
    items.sort((a, b) => {
      const av = sortBy === 'success_rate'
        ? (a.request_count > 0 ? a.success_count / a.request_count : 0)
        : a[sortBy]
      const bv = sortBy === 'success_rate'
        ? (b.request_count > 0 ? b.success_count / b.request_count : 0)
        : b[sortBy]
      return sortOrder === 'desc' ? (av > bv ? -1 : 1) : (av < bv ? -1 : 1)
    })
    return items
  }, [filtered, sortBy, sortOrder])

  const totals = stats.reduce(
    (acc, s) => ({
      requests: acc.requests + s.request_count,
      success: acc.success + s.success_count,
      failure: acc.failure + s.failure_count,
      input: acc.input + s.input_tokens,
      output: acc.output + s.output_tokens,
      cost: acc.cost + s.total_cost,
    }),
    { requests: 0, success: 0, failure: 0, input: 0, output: 0, cost: 0 },
  )

  const successRate = totals.requests > 0
    ? ((totals.success / totals.requests) * 100).toFixed(1)
    : '-'

  const handleSort = (col: SortField) => {
    if (sortBy === col) {
      setSortOrder((o) => (o === 'desc' ? 'asc' : 'desc'))
    } else {
      setSortBy(col)
      setSortOrder('desc')
    }
  }

  const SortButton = ({ col, label }: { col: SortField; label: string }) => (
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
        title="渠道统计"
        subtitle="查看每个渠道的请求量、成功率和成本"
      />

      {/* 汇总卡片 */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
        <SummaryCard label="总请求" value={formatNumber(totals.requests)} />
        <SummaryCard label="成功率" value={`${successRate}%`} />
        <SummaryCard label="总 Token" value={formatNumber(totals.input + totals.output)} />
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
          searchPlaceholder="搜索渠道名称..."
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
            <DistributionPieChart data={pieData} loading={isLoading} />
          </div>
          <div className="rounded-2xl border bg-card p-5">
            <ChannelCompareChart data={compareData} loading={isLoading} />
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
                  <th className="text-left px-4 py-3 font-medium">渠道</th>
                  <th className="text-right px-4 py-3 font-medium"><SortButton col="request_count" label="请求量" /></th>
                  <th className="text-right px-4 py-3 font-medium">成功</th>
                  <th className="text-right px-4 py-3 font-medium">失败</th>
                  <th className="text-right px-4 py-3 font-medium"><SortButton col="success_rate" label="成功率" /></th>
                  <th className="text-right px-4 py-3 font-medium"><SortButton col="total_cost" label="成本" /></th>
                </tr>
              </thead>
              <tbody>
                <EmptyState
                  loading={isLoading}
                  isEmpty={!isLoading && sorted.length === 0}
                  loadingText="加载中..."
                  emptyText={search ? '没有匹配的渠道' : '暂无统计数据'}
                  colSpan={6}
                />
                {!isLoading && sorted.map((row) => {
                  const rate = row.request_count > 0
                    ? ((row.success_count / row.request_count) * 100).toFixed(1)
                    : '-'
                  return (
                    <tr
                      key={row.channel_id}
                      className="border-b last:border-0 hover:bg-muted/30 transition-colors"
                    >
                      <td className="px-4 py-3 font-medium">{row.channel_name || '未知'}</td>
                      <td className="px-4 py-3 text-right tabular-nums">{formatNumber(row.request_count)}</td>
                      <td className="px-4 py-3 text-right tabular-nums text-green-600">{formatNumber(row.success_count)}</td>
                      <td className="px-4 py-3 text-right tabular-nums text-destructive">{formatNumber(row.failure_count)}</td>
                      <td className="px-4 py-3 text-right tabular-nums">{rate}%</td>
                      <td className="px-4 py-3 text-right tabular-nums">{formatCost(row.total_cost)}</td>
                    </tr>
                  )
                })}
              </tbody>
            </table>
          </div>
        </div>
      )}
    </div>
  )
}
