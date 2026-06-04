import {
  PieChart, Pie, Cell, Tooltip,
} from 'recharts'
import { EmptyState } from '@/components/common'
import { PIE_COLORS, tooltipStyle } from './styles'

interface PieDataItem {
  name: string
  value: number
  cost: number
}

interface DistributionPieChartProps {
  data: PieDataItem[]
  title?: string
  loading?: boolean
  onNameClick?: (name: string) => void
}

const fmt = (n: number) => n.toLocaleString()
const fmtCost = (n: number) => n.toFixed(4)

export function DistributionPieChart({
  data,
  title = '请求量分布',
  loading,
  onNameClick,
}: DistributionPieChartProps) {
  return (
    <div>
      <h3 className="text-xs font-medium text-muted-foreground mb-3">{title}</h3>
      {data.length > 0 ? (
        <div className="flex gap-4">
          <div className="w-1/2">
            <PieChart width={200} height={200}>
              <Pie
                data={data.slice(0, 6)}
                dataKey="value"
                nameKey="name"
                outerRadius={80}
                innerRadius={40}
                labelLine={false}
                strokeWidth={0}
              >
                {data.slice(0, 6).map((_, i) => (
                  <Cell key={i} fill={PIE_COLORS[i % PIE_COLORS.length]} />
                ))}
              </Pie>
              <Tooltip formatter={(v) => fmt(Number(v))} contentStyle={tooltipStyle} labelStyle={{ color: 'var(--color-muted-foreground)' }} />
            </PieChart>
          </div>
          <div className="w-1/2 overflow-auto max-h-[200px]">
            <table className="w-full text-xs">
              <thead>
                <tr className="border-b text-muted-foreground">
                  <th className="text-left py-1 font-medium">#</th>
                  <th className="text-left py-1 font-medium">名称</th>
                  <th className="text-right py-1 font-medium">请求</th>
                  <th className="text-right py-1 font-medium">成本</th>
                </tr>
              </thead>
              <tbody>
                {data.map((m, i) => (
                  <tr key={m.name} className="border-b last:border-0">
                    <td className="py-1 text-muted-foreground">
                      <span className="inline-block w-2 h-2 rounded-full mr-1" style={{ backgroundColor: PIE_COLORS[i % PIE_COLORS.length] }} />
                      {i + 1}
                    </td>
                    <td className="py-1 font-medium max-w-[140px]">
                      {onNameClick ? (
                        <button
                          onClick={() => onNameClick(m.name)}
                          className="truncate text-left text-primary hover:underline w-full"
                          title={m.name}
                        >
                          {m.name}
                        </button>
                      ) : (
                        <span className="truncate block" title={m.name}>{m.name}</span>
                      )}
                    </td>
                    <td className="py-1 text-right">{fmt(m.value)}</td>
                    <td className="py-1 text-right">${fmtCost(m.cost)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      ) : (
        <EmptyState loading={loading} isEmpty={!loading} emptyText="暂无数据" loadingText="加载图表数据..." standalone />
      )}
    </div>
  )
}
