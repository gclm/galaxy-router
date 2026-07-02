import { useState, useMemo } from 'react'
import { useStatsApiKeys } from '@/api/query-hooks'
import { PageHeader, FilterBar, SummaryCard, ViewToggle, DataTable } from '@/components/common'
import { DistributionPieChart, CostBarChart } from '@/components/charts'
import { formatNumber, formatCost } from '@/lib/utils'

export function ApiKeyStats() {
  const [showChart, setShowChart] = useState(true)
  const [showTable, setShowTable] = useState(true)
  const [search, setSearch] = useState('')
  const [days, setDays] = useState(7)

  const { data, isLoading, refetch } = useStatsApiKeys(days)
  const stats = useMemo(() => data ?? [], [data])

  const chartData = useMemo(() =>
    stats.map((s) => ({ name: s.api_key_name, value: s.request_count, cost: s.total_cost }))
  , [stats])

  const filtered = useMemo(() => {
    if (!showTable || !search) return stats
    return stats.filter((s) =>
      s.api_key_name.toLowerCase().includes(search.toLowerCase()) ||
      s.api_key_id.toLowerCase().includes(search.toLowerCase()),
    )
  }, [stats, search, showTable])

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

  return (
    <div className="space-y-4">
      <PageHeader
        title="Key 用量统计"
        subtitle="查看每个 API Key 的请求量、Token 消耗和成本"
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
          searchPlaceholder="搜索 Key 名称..."
          onRefresh={() => refetch()}
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
            <CostBarChart data={chartData} title="Key 成本 Top 8" loading={isLoading} />
          </div>
        </div>
      )}

      {/* 表格区域 */}
      {showTable && (
        <DataTable
          columns={[
            { header: 'Key 名称' },
            { header: '请求量', align: 'right' },
            { header: '成功', align: 'right' },
            { header: '失败', align: 'right' },
            { header: '成功率', align: 'right' },
            { header: '输入 Token', align: 'right' },
            { header: '输出 Token', align: 'right' },
            { header: '成本', align: 'right' },
          ]}
          loading={isLoading}
          isEmpty={!isLoading && filtered.length === 0}
          emptyText={search ? '没有匹配的 Key' : '暂无统计数据'}
        >
          {filtered.map((row) => {
            const rate = row.request_count > 0
              ? ((row.success_count / row.request_count) * 100).toFixed(1)
              : '-'
            return (
              <tr
                key={row.api_key_id}
                className="border-b last:border-0 hover:bg-muted/30 transition-colors"
              >
                <td className="px-4 py-3">
                  <p className="font-medium">{row.api_key_name || '未知'}</p>
                  <p className="text-xs text-muted-foreground font-mono">{row.api_key_id.slice(0, 12)}...</p>
                </td>
                <td className="px-4 py-3 text-right tabular-nums">{formatNumber(row.request_count)}</td>
                <td className="px-4 py-3 text-right tabular-nums text-green-600">{formatNumber(row.success_count)}</td>
                <td className="px-4 py-3 text-right tabular-nums text-destructive">{formatNumber(row.failure_count)}</td>
                <td className="px-4 py-3 text-right tabular-nums">{rate}%</td>
                <td className="px-4 py-3 text-right tabular-nums text-muted-foreground">{formatNumber(row.input_tokens)}</td>
                <td className="px-4 py-3 text-right tabular-nums text-muted-foreground">{formatNumber(row.output_tokens)}</td>
                <td className="px-4 py-3 text-right tabular-nums">{formatCost(row.total_cost)}</td>
              </tr>
            )
          })}
        </DataTable>
      )}
    </div>
  )
}
