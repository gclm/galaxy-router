import {
  BarChart, Bar, XAxis, YAxis, Tooltip, ResponsiveContainer, CartesianGrid, Legend,
} from 'recharts'
import { EmptyState } from '@/components/common'
import { C_GREEN, C_ROSE, tooltipStyle, tickStyle, legendStyle } from './styles'

interface ChannelCompareChartProps {
  data: Array<{ name: string; success: number; failure: number }>
  topN?: number
  loading?: boolean
}

const fmt = (n: number) => n.toLocaleString()

export function ChannelCompareChart({
  data,
  topN = 8,
  loading,
}: ChannelCompareChartProps) {
  const sorted = [...data]
    .sort((a, b) => (b.success + b.failure) - (a.success + a.failure))
    .slice(0, topN)

  return (
    <div>
      <h3 className="text-xs font-medium text-muted-foreground mb-3">渠道成功/失败对比</h3>
      {sorted.length > 0 ? (
        <ResponsiveContainer width="100%" height={200}>
          <BarChart data={sorted} layout="vertical">
            <CartesianGrid strokeDasharray="3 3" stroke="var(--color-border)" horizontal={false} />
            <XAxis type="number" tick={tickStyle} axisLine={false} tickLine={false} />
            <YAxis type="category" dataKey="name" tick={tickStyle} axisLine={false} tickLine={false} width={80} />
            <Tooltip formatter={(v) => fmt(Number(v))} contentStyle={tooltipStyle} labelStyle={{ color: 'var(--color-muted-foreground)' }} />
            <Legend wrapperStyle={legendStyle} />
            <Bar dataKey="success" name="成功" fill={C_GREEN} radius={[0, 2, 2, 0]} stackId="a" />
            <Bar dataKey="failure" name="失败" fill={C_ROSE} radius={[0, 2, 2, 0]} stackId="a" />
          </BarChart>
        </ResponsiveContainer>
      ) : (
        <EmptyState loading={loading} isEmpty={!loading} emptyText="暂无渠道数据" loadingText="加载图表数据..." standalone />
      )}
    </div>
  )
}
