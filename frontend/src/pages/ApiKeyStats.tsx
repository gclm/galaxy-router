import { useState } from 'react'
import { useStatsApiKeys } from '@/api/query-hooks'
import { PageHeader } from '@/components/common/PageHeader'
import { EmptyState } from '@/components/common/EmptyState'
import { formatNumber, formatCost } from '@/lib/utils'
import {
  BarChart3,
} from 'lucide-react'

export function ApiKeyStats() {
  const [search, setSearch] = useState('')
  const [days, setDays] = useState(7)

  const { data, isLoading } = useStatsApiKeys(days)
  const stats = data ?? []

  const filtered = search
    ? stats.filter((s) =>
        s.api_key_name.toLowerCase().includes(search.toLowerCase()) ||
        s.api_key_id.toLowerCase().includes(search.toLowerCase()),
      )
    : stats

  const totals = filtered.reduce(
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

      {/* 筛选栏 */}
      <div className="flex items-center gap-3 flex-wrap">
        <div className="relative flex-1 min-w-[200px] max-w-sm">
          <BarChart3 className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
          <input
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="搜索 Key 名称..."
            className="input pl-9"
          />
        </div>
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
      </div>

      {/* 表格 */}
      <div className="rounded-2xl border bg-card overflow-hidden">
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b bg-muted/50">
                <th className="text-left px-4 py-3 font-medium">Key 名称</th>
                <th className="text-right px-4 py-3 font-medium">请求量</th>
                <th className="text-right px-4 py-3 font-medium">成功</th>
                <th className="text-right px-4 py-3 font-medium">失败</th>
                <th className="text-right px-4 py-3 font-medium">成功率</th>
                <th className="text-right px-4 py-3 font-medium">输入 Token</th>
                <th className="text-right px-4 py-3 font-medium">输出 Token</th>
                <th className="text-right px-4 py-3 font-medium">成本</th>
              </tr>
            </thead>
            <tbody>
              <EmptyState
                loading={isLoading}
                isEmpty={!isLoading && filtered.length === 0}
                loadingText="加载中..."
                emptyText={search ? '没有匹配的 Key' : '暂无统计数据'}
                colSpan={8}
              />
              {!isLoading && filtered.map((row) => {
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
            </tbody>
          </table>
        </div>
      </div>
    </div>
  )
}

function SummaryCard({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-xl bg-muted/30 border border-border p-4">
      <p className="text-xs font-medium text-muted-foreground">{label}</p>
      <p className="text-lg font-bold tracking-tight mt-1">{value}</p>
    </div>
  )
}
