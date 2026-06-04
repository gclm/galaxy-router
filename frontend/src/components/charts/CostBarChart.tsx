import {
  BarChart, Bar, XAxis, YAxis, Tooltip, ResponsiveContainer, CartesianGrid,
} from 'recharts'
import { EmptyState } from '@/components/common'
import { C_AMBER, tooltipStyle, tickStyle } from './styles'

interface CostBarChartProps {
  data: Array<{ name: string; cost: number }>
  title?: string
  topN?: number
  loading?: boolean
}

const fmtCost = (n: number) => `$${n.toFixed(4)}`

export function CostBarChart({
  data,
  title = '成本 Top 8',
  topN = 8,
  loading,
}: CostBarChartProps) {
  const sorted = [...data]
    .sort((a, b) => b.cost - a.cost)
    .slice(0, topN)

  return (
    <div>
      <h3 className="text-xs font-medium text-muted-foreground mb-3">{title}</h3>
      {sorted.length > 0 ? (
        <ResponsiveContainer width="100%" height={200}>
          <BarChart data={sorted} layout="vertical">
            <defs>
              <linearGradient id="grad-cost-bar" x1="0" y1="0" x2="1" y2="0">
                <stop offset="0%" stopColor={C_AMBER} stopOpacity={0.5} />
                <stop offset="100%" stopColor={C_AMBER} stopOpacity={0.9} />
              </linearGradient>
            </defs>
            <CartesianGrid strokeDasharray="3 3" stroke="var(--color-border)" horizontal={false} />
            <XAxis type="number" tick={tickStyle} axisLine={false} tickLine={false} />
            <YAxis type="category" dataKey="name" tick={tickStyle} axisLine={false} tickLine={false} width={80} />
            <Tooltip formatter={(v) => fmtCost(Number(v))} contentStyle={tooltipStyle} labelStyle={{ color: 'var(--color-muted-foreground)' }} />
            <Bar dataKey="cost" fill="url(#grad-cost-bar)" radius={[0, 4, 4, 0]} />
          </BarChart>
        </ResponsiveContainer>
      ) : (
        <EmptyState loading={loading} isEmpty={!loading} emptyText="暂无成本数据" loadingText="加载图表数据..." standalone />
      )}
    </div>
  )
}
