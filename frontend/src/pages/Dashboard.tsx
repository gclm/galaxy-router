import { useState, useMemo, useRef, useCallback, useEffect } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import type { StatsParams } from '@/api'
import type { SystemInfo, DailyStats, ModelStats, ChannelStats } from '@/api/types'
import { useStatsOverview, useSystemInfo, useStatsDaily, useStatsModels, useStatsChannels, useStatsLatency } from '@/api/query-hooks'
import { useAutoRefresh } from '@/hooks/useAutoRefresh'
import { StatCard, AnimatedNumber, EmptyState } from '@/components/common'
import {
  Activity, MessageSquare, Coins,
  Cpu, Clock, Radio, Layers, Key, Calendar, ChevronDown,
  ShieldCheck, ShieldAlert,
} from 'lucide-react'
import {
  BarChart, Bar, XAxis, YAxis, Tooltip, ResponsiveContainer,
  AreaChart, Area, CartesianGrid, PieChart, Pie, Cell, Legend,
} from 'recharts'

const C_BLUE = 'var(--color-chart-1)'
const C_GREEN = 'var(--color-chart-2)'
const C_AMBER = 'var(--color-chart-3)'
const C_VIOLET = 'var(--color-chart-4)'
const C_ROSE = 'var(--color-chart-5)'

const PIE_COLORS = [C_BLUE, C_GREEN, C_AMBER, C_VIOLET, C_ROSE]

const tooltipStyle: React.CSSProperties = {
  backgroundColor: 'var(--color-popover)',
  border: '1px solid var(--color-border)',
  borderRadius: '0.75rem',
  color: 'var(--color-popover-foreground)',
  fontSize: 12,
  boxShadow: '0 4px 16px rgba(0,0,0,0.12)',
  padding: '8px 12px',
}

const tickStyle = { fill: 'var(--color-muted-foreground)', fontSize: 11 }
const legendStyle = { fontSize: 11, color: 'var(--color-foreground)' }

const RANGE_TABS = [
  { label: '今天', days: 1 },
  { label: '7天', days: 7 },
  { label: '30天', days: 30 },
  { label: '90天', days: 90 },
] as const

function formatUptime(secs: number): string {
  const d = Math.floor(secs / 86400)
  const h = Math.floor((secs % 86400) / 3600)
  const m = Math.floor((secs % 3600) / 60)
  if (d > 0) return `${d}天 ${h}小时`
  if (h > 0) return `${h}小时 ${m}分钟`
  return `${m}分钟`
}

const fmt = (n: number) => n.toLocaleString()
const fmtCost = (n: number) => n.toFixed(4)
const fmtTokens = (n: number) => {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`
  return n.toLocaleString()
}

export function Dashboard() {
  const qc = useQueryClient()

  const overviewQuery = useStatsOverview()
  const systemQuery = useSystemInfo()

  const [activeRange, setActiveRange] = useState(1)
  const [customStart, setCustomStart] = useState('')
  const [customEnd, setCustomEnd] = useState('')
  const [customMode, setCustomMode] = useState(false)

  const chartParams = useMemo<StatsParams>(() => {
    if (customMode && customStart && customEnd) {
      return { start_date: customStart, end_date: customEnd }
    }
    return { days: activeRange }
  }, [customMode, customStart, customEnd, activeRange])

  const dailyQuery = useStatsDaily(chartParams)
  const modelsQuery = useStatsModels(chartParams)
  const channelsQuery = useStatsChannels(chartParams)
  const latencyQuery = useStatsLatency(chartParams)

  // Auto-refresh (default 60s, persisted)
  const refetchAll = useCallback(() => {
    void qc.invalidateQueries({ queryKey: ['stats'] })
    void qc.invalidateQueries({ queryKey: ['system-info'] })
  }, [qc])

  useAutoRefresh({
    refetch: refetchAll,
    defaultInterval: 60,
    storageKey: 'dashboard-refresh',
  })

  const handleRangeTab = (days: number) => {
    setCustomMode(false)
    setActiveRange(days)
  }

  const handleCustomApply = () => {
    if (customStart && customEnd) setCustomMode(true)
  }

  const sortedDaily = useMemo(() =>
    [...(dailyQuery.data ?? [])].sort((a, b) => a.date.localeCompare(b.date))
  , [dailyQuery.data])

  const daily = dailyQuery.data ?? []

  const summary = useMemo(() => {
    const requests = daily.reduce((s, d) => s + d.request_count, 0)
    const success = daily.reduce((s, d) => s + d.success_count, 0)
    const failure = daily.reduce((s, d) => s + d.failure_count, 0)
    const inputTokens = daily.reduce((s, d) => s + d.input_tokens, 0)
    const outputTokens = daily.reduce((s, d) => s + d.output_tokens, 0)
    const cacheReadTokens = daily.reduce((s, d) => s + d.cache_read_tokens, 0)
    const cacheCreationTokens = daily.reduce((s, d) => s + d.cache_creation_tokens, 0)
    const cost = daily.reduce((s, d) => s + d.total_cost, 0)
    const successRate = requests > 0 ? ((success / requests) * 100) : 0
    return { requests, success, failure, inputTokens, outputTokens, cacheReadTokens, cacheCreationTokens, cost, successRate }
  }, [daily])

  const latency = latencyQuery.data
  const p50 = latency?.p50_latency_ms ?? 0
  const p95 = latency?.p95_latency_ms ?? 0
  const p99 = latency?.p99_latency_ms ?? 0

  const total = overviewQuery.data
    ? { requests: overviewQuery.data.total_requests, tokens: overviewQuery.data.total_input_tokens + overviewQuery.data.total_output_tokens, cost: overviewQuery.data.total_cost }
    : { requests: 0, tokens: 0, cost: 0 }

  const rangeLabel = customMode ? `${customStart} ~ ${customEnd}` : RANGE_TABS.find(t => t.days === activeRange)?.label ?? ''

  // Initial page load spinner
  if (overviewQuery.isLoading) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="flex items-center gap-2 text-muted-foreground">
          <div className="h-4 w-4 animate-spin rounded-full border-2 border-current border-t-transparent" />
          <span>加载中...</span>
        </div>
      </div>
    )
  }

  const systemInfo = systemQuery.data as SystemInfo | undefined

  return (
    <div className="space-y-4">
      {/* 系统信息卡片 */}
      {systemInfo && (
        <div className="grid grid-cols-3 sm:grid-cols-6 gap-3">
          {[
            { label: '版本', value: `v${systemInfo.version}`, icon: Cpu, color: 'from-blue-500 to-blue-600' },
            { label: '运行时间', value: formatUptime(systemInfo.uptime_secs), icon: Clock, color: 'from-indigo-500 to-indigo-600' },
            { label: '渠道', value: `${systemInfo.channel_count} 个`, icon: Radio, color: 'from-violet-500 to-violet-600' },
            { label: '分组', value: `${systemInfo.group_count} 个`, icon: Layers, color: 'from-purple-500 to-purple-600' },
            { label: 'API Key', value: `${systemInfo.api_key_count} 个`, icon: Key, color: 'from-fuchsia-500 to-fuchsia-600' },
            { label: '状态', value: '运行中', icon: Activity, color: 'from-emerald-500 to-emerald-600', running: true },
          ].map((item) => (
            <div key={item.label} className="rounded-xl border bg-card p-3 flex items-center gap-2.5">
              <div className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-gradient-to-br ${item.color} text-white shadow-sm`}>
                {item.running ? (
                  <span className="relative flex h-2.5 w-2.5">
                    <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-white/60 opacity-75" />
                    <span className="relative inline-flex h-2.5 w-2.5 rounded-full bg-white" />
                  </span>
                ) : (
                  <item.icon className="h-4 w-4" />
                )}
              </div>
              <div className="min-w-0">
                <p className="text-[11px] text-muted-foreground leading-tight">{item.label}</p>
                <p className="text-sm font-medium truncate leading-tight mt-0.5">{item.value}</p>
              </div>
            </div>
          ))}
        </div>
      )}

      {/* 趋势分析 */}
      <div className="rounded-2xl border bg-card">
        <div className="flex flex-wrap items-center justify-between gap-3 border-b px-5 py-3.5">
          <h2 className="text-sm font-semibold">趋势分析</h2>
          <RangePicker
            activeRange={activeRange}
            customMode={customMode}
            customStart={customStart}
            customEnd={customEnd}
            onSelectRange={handleRangeTab}
            onCustomStartChange={setCustomStart}
            onCustomEndChange={setCustomEnd}
            onCustomApply={handleCustomApply}
          />
        </div>

        {/* KPI 卡片 */}
        <div className="grid grid-cols-2 md:grid-cols-5 gap-4 px-5 pt-4">
          <StatCard
            label="请求数"
            value={<AnimatedNumber value={summary.requests} format={fmt} />}
            subtitle={
              <>
                成功 <AnimatedNumber value={summary.success} format={fmt} /> / 失败 <AnimatedNumber value={summary.failure} format={fmt} />
              </>
            }
            icon={Activity}
            gradient="from-blue-500 to-blue-600"
          />
          <StatCard
            label="Token 用量"
            value={fmtTokens(summary.inputTokens + summary.outputTokens)}
            subtitle={
              <>
                入 {fmtTokens(summary.inputTokens)} · 出 {fmtTokens(summary.outputTokens)}
                {(summary.cacheReadTokens + summary.cacheCreationTokens) > 0 && (
                  <> · 缓存读 {fmtTokens(summary.cacheReadTokens)} · 缓存写 {fmtTokens(summary.cacheCreationTokens)}</>
                )}
              </>
            }
            icon={MessageSquare}
            gradient="from-violet-500 to-violet-600"
          />
          <StatCard
            label="成本"
            value={`$${fmtCost(summary.cost)}`}
            subtitle={<>累计 ${`$${fmtCost(total.cost)}`}</>}
            icon={Coins}
            gradient="from-amber-500 to-amber-600"
          />
          <StatCard
            label="成功率"
            value={`${summary.successRate.toFixed(1)}%`}
            subtitle={
              <>
                {rangeLabel} · 累计 <AnimatedNumber value={total.requests} format={fmt} /> 次
              </>
            }
            icon={summary.successRate >= 95 ? ShieldCheck : ShieldAlert}
            gradient="from-emerald-500 to-emerald-600"
          />
          <StatCard
            label="延迟"
            value={`${p50.toFixed(0)}ms`}
            subtitle={
              <>
                P50 {p50.toFixed(0)}ms · P95 {p95.toFixed(0)}ms · P99 {p99.toFixed(0)}ms
              </>
            }
            icon={Clock}
            gradient="from-rose-500 to-rose-600"
          />
        </div>

        {/* 图表区域 */}
        <div className="p-5 space-y-5">
          <div className="grid gap-5 md:grid-cols-2">
            <AreaChartCard title="请求趋势" data={sortedDaily} dataKey="request_count" stroke={C_BLUE} emptyText="暂无请求数据" loading={dailyQuery.isLoading} />
            <TokenAreaChart data={sortedDaily} loading={dailyQuery.isLoading} />
          </div>

          <div className="grid gap-5 md:grid-cols-2">
            <ModelDistributionChart models={modelsQuery.data ?? []} loading={modelsQuery.isLoading} />
            <ChannelBarChart channels={channelsQuery.data ?? []} loading={channelsQuery.isLoading} />
          </div>

          <CostBarChart data={sortedDaily} loading={dailyQuery.isLoading} />
        </div>
      </div>
    </div>
  )
}

/* ---- RangePicker ---- */

function RangePicker({
  activeRange,
  customMode,
  customStart,
  customEnd,
  onSelectRange,
  onCustomStartChange,
  onCustomEndChange,
  onCustomApply,
}: {
  activeRange: number
  customMode: boolean
  customStart: string
  customEnd: string
  onSelectRange: (days: number) => void
  onCustomStartChange: (v: string) => void
  onCustomEndChange: (v: string) => void
  onCustomApply: () => void
}) {
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false)
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [open])

  const currentLabel = customMode && customStart && customEnd
    ? `${customStart} ~ ${customEnd}`
    : RANGE_TABS.find(t => t.days === activeRange)?.label ?? '选择时间'

  return (
    <div className="relative" ref={ref}>
      <button
        onClick={() => setOpen(!open)}
        className="flex items-center gap-1.5 rounded-lg border bg-background px-3 py-1.5 text-xs font-medium text-foreground shadow-sm hover:bg-accent transition-colors"
      >
        <Calendar className="h-3.5 w-3.5 text-muted-foreground" />
        {currentLabel}
        <ChevronDown className={`h-3 w-3 text-muted-foreground transition-transform ${open ? 'rotate-180' : ''}`} />
      </button>

      {open && (
        <div className="absolute right-0 top-full mt-1.5 z-50 w-72 rounded-xl border bg-popover p-3 shadow-lg">
          <div className="grid grid-cols-4 gap-1.5 mb-3">
            {RANGE_TABS.map((tab) => (
              <button
                key={tab.days}
                onClick={() => { onSelectRange(tab.days); setOpen(false) }}
                className={`rounded-lg px-2 py-1.5 text-xs font-medium transition-colors ${
                  !customMode && activeRange === tab.days
                    ? 'bg-primary text-primary-foreground'
                    : 'text-muted-foreground hover:bg-muted'
                }`}
              >
                {tab.label}
              </button>
            ))}
          </div>
          <div className="border-t mb-3" />
          <p className="text-[11px] text-muted-foreground mb-2">自定义范围</p>
          <div className="flex items-center gap-1.5 mb-2.5">
            <input
              type="date"
              value={customStart}
              max={customEnd || undefined}
              onChange={(e) => { onCustomStartChange(e.target.value); if (!customEnd) onCustomEndChange(e.target.value) }}
              className="input h-7 flex-1 text-xs py-0 px-2"
            />
            <span className="text-xs text-muted-foreground">~</span>
            <input
              type="date"
              value={customEnd}
              min={customStart || undefined}
              onChange={(e) => onCustomEndChange(e.target.value)}
              className="input h-7 flex-1 text-xs py-0 px-2"
            />
          </div>
          <button
            onClick={() => { onCustomApply(); setOpen(false) }}
            disabled={!customStart || !customEnd}
            className="w-full rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground disabled:opacity-50"
          >
            应用
          </button>
        </div>
      )}
    </div>
  )
}

/* ---- 通用渐变面积图 ---- */

function AreaChartCard({ title, data, dataKey, stroke, emptyText, loading }: {
  title: string
  data: DailyStats[]
  dataKey: string
  stroke: string
  emptyText: string
  loading?: boolean
}) {
  const id = dataKey.replace(/_/g, '-')
  return (
    <div>
      <h3 className="text-xs font-medium text-muted-foreground mb-3">{title}</h3>
      {data.length > 0 ? (
        <ResponsiveContainer width="100%" height={220}>
          <AreaChart data={data}>
            <defs>
              <linearGradient id={`grad-${id}`} x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stopColor={stroke} stopOpacity={0.25} />
                <stop offset="100%" stopColor={stroke} stopOpacity={0.02} />
              </linearGradient>
            </defs>
            <CartesianGrid strokeDasharray="3 3" stroke="var(--color-border)" vertical={false} />
            <XAxis dataKey="date" tick={tickStyle} axisLine={false} tickLine={false} />
            <YAxis tick={tickStyle} axisLine={false} tickLine={false} />
            <Tooltip formatter={(v) => fmt(Number(v))} contentStyle={tooltipStyle} labelStyle={{ color: 'var(--color-muted-foreground)', marginBottom: 4 }} />
            <Area type="monotone" dataKey={dataKey} stroke={stroke} strokeWidth={2} fill={`url(#grad-${id})`} dot={false} />
          </AreaChart>
        </ResponsiveContainer>
      ) : (
        <EmptyState loading={loading} isEmpty={!loading} emptyText={emptyText} loadingText="加载图表数据..." standalone />
      )}
    </div>
  )
}

/* ---- Token 趋势（多系列面积图） ---- */

function TokenAreaChart({ data, loading }: { data: DailyStats[]; loading?: boolean }) {
  const hasCache = data.some(d => (d.cache_read_tokens + d.cache_creation_tokens) > 0)

  return (
    <div>
      <h3 className="text-xs font-medium text-muted-foreground mb-3">Token 用量趋势</h3>
      {data.length > 0 ? (
        <ResponsiveContainer width="100%" height={220}>
          <AreaChart data={data}>
            <defs>
              <linearGradient id="grad-input" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stopColor={C_BLUE} stopOpacity={0.2} />
                <stop offset="100%" stopColor={C_BLUE} stopOpacity={0.02} />
              </linearGradient>
              <linearGradient id="grad-output" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stopColor={C_GREEN} stopOpacity={0.2} />
                <stop offset="100%" stopColor={C_GREEN} stopOpacity={0.02} />
              </linearGradient>
            </defs>
            <CartesianGrid strokeDasharray="3 3" stroke="var(--color-border)" vertical={false} />
            <XAxis dataKey="date" tick={tickStyle} axisLine={false} tickLine={false} />
            <YAxis tick={tickStyle} axisLine={false} tickLine={false} />
            <Tooltip formatter={(v) => fmt(Number(v))} contentStyle={tooltipStyle} labelStyle={{ color: 'var(--color-muted-foreground)', marginBottom: 4 }} />
            <Legend wrapperStyle={legendStyle} />
            <Area type="monotone" dataKey="input_tokens" stroke={C_BLUE} strokeWidth={2} fill="url(#grad-input)" dot={false} name="输入" />
            <Area type="monotone" dataKey="output_tokens" stroke={C_GREEN} strokeWidth={2} fill="url(#grad-output)" dot={false} name="输出" />
            {hasCache && (
              <>
                <Area type="monotone" dataKey="cache_read_tokens" stroke={C_AMBER} strokeWidth={1.5} fill="none" dot={false} strokeDasharray="4 2" name="缓存读" />
                <Area type="monotone" dataKey="cache_creation_tokens" stroke={C_VIOLET} strokeWidth={1.5} fill="none" dot={false} strokeDasharray="4 2" name="缓存写" />
              </>
            )}
          </AreaChart>
        </ResponsiveContainer>
      ) : (
        <EmptyState loading={loading} isEmpty={!loading} emptyText="暂无 Token 数据" loadingText="加载图表数据..." standalone />
      )}
    </div>
  )
}

/* ---- 模型分布（饼图 + 表格） ---- */

function ModelDistributionChart({ models, loading }: { models: ModelStats[]; loading?: boolean }) {
  return (
    <div>
      <h3 className="text-xs font-medium text-muted-foreground mb-3">模型分布</h3>
      {models.length > 0 ? (
        <div className="flex gap-4">
          <div className="w-1/2">
            <ResponsiveContainer width="100%" height={200}>
              <PieChart>
                <Pie data={models.slice(0, 6)} dataKey="request_count" nameKey="model" outerRadius={80} innerRadius={40} labelLine={false} strokeWidth={0}>
                  {models.slice(0, 6).map((_, i) => (
                    <Cell key={i} fill={PIE_COLORS[i % PIE_COLORS.length]} />
                  ))}
                </Pie>
                <Tooltip formatter={(v) => fmt(Number(v))} contentStyle={tooltipStyle} labelStyle={{ color: 'var(--color-muted-foreground)' }} />
              </PieChart>
            </ResponsiveContainer>
          </div>
          <div className="w-1/2 overflow-auto max-h-[200px]">
            <table className="w-full text-xs">
              <thead>
                <tr className="border-b text-muted-foreground">
                  <th className="text-left py-1 font-medium">#</th>
                  <th className="text-left py-1 font-medium">模型</th>
                  <th className="text-right py-1 font-medium">请求</th>
                  <th className="text-right py-1 font-medium">成本</th>
                </tr>
              </thead>
              <tbody>
                {models.map((m, i) => (
                  <tr key={m.model} className="border-b last:border-0">
                    <td className="py-1 text-muted-foreground">
                      <span className="inline-block w-2 h-2 rounded-full mr-1" style={{ backgroundColor: PIE_COLORS[i % PIE_COLORS.length] }} />
                      {i + 1}
                    </td>
                    <td className="py-1 font-medium max-w-[140px] truncate" title={m.model}>{m.model}</td>
                    <td className="py-1 text-right">{fmt(m.request_count)}</td>
                    <td className="py-1 text-right">${fmtCost(m.total_cost)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      ) : (
        <EmptyState loading={loading} isEmpty={!loading} emptyText="暂无模型数据" loadingText="加载图表数据..." standalone />
      )}
    </div>
  )
}

/* ---- 渠道请求量（柱状图） ---- */

function ChannelBarChart({ channels, loading }: { channels: ChannelStats[]; loading?: boolean }) {
  return (
    <div>
      <h3 className="text-xs font-medium text-muted-foreground mb-3">渠道请求量</h3>
      {channels.length > 0 ? (
        <ResponsiveContainer width="100%" height={200}>
          <BarChart data={channels.slice(0, 8)}>
            <defs>
              <linearGradient id="grad-bar-channel" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stopColor={C_GREEN} stopOpacity={0.9} />
                <stop offset="100%" stopColor={C_GREEN} stopOpacity={0.5} />
              </linearGradient>
            </defs>
            <CartesianGrid strokeDasharray="3 3" stroke="var(--color-border)" vertical={false} />
            <XAxis dataKey="channel_name" tick={tickStyle} axisLine={false} tickLine={false} />
            <YAxis tick={tickStyle} axisLine={false} tickLine={false} />
            <Tooltip formatter={(v) => fmt(Number(v))} contentStyle={tooltipStyle} labelStyle={{ color: 'var(--color-muted-foreground)' }} />
            <Bar dataKey="request_count" fill="url(#grad-bar-channel)" radius={[4, 4, 0, 0]} />
          </BarChart>
        </ResponsiveContainer>
      ) : (
        <EmptyState loading={loading} isEmpty={!loading} emptyText="暂无渠道数据" loadingText="加载图表数据..." standalone />
      )}
    </div>
  )
}

/* ---- 每日成本（柱状图） ---- */

function CostBarChart({ data, loading }: { data: DailyStats[]; loading?: boolean }) {
  return (
    <div>
      <h3 className="text-xs font-medium text-muted-foreground mb-3">每日成本</h3>
      {data.length > 0 ? (
        <ResponsiveContainer width="100%" height={200}>
          <BarChart data={data}>
            <defs>
              <linearGradient id="grad-bar-cost" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stopColor={C_AMBER} stopOpacity={0.9} />
                <stop offset="100%" stopColor={C_AMBER} stopOpacity={0.5} />
              </linearGradient>
            </defs>
            <CartesianGrid strokeDasharray="3 3" stroke="var(--color-border)" vertical={false} />
            <XAxis dataKey="date" tick={tickStyle} axisLine={false} tickLine={false} />
            <YAxis tick={tickStyle} axisLine={false} tickLine={false} />
            <Tooltip formatter={(v) => `$${fmtCost(Number(v))}`} contentStyle={tooltipStyle} labelStyle={{ color: 'var(--color-muted-foreground)' }} />
            <Bar dataKey="total_cost" fill="url(#grad-bar-cost)" radius={[4, 4, 0, 0]} />
          </BarChart>
        </ResponsiveContainer>
      ) : (
        <EmptyState loading={loading} isEmpty={!loading} emptyText="暂无成本数据" loadingText="加载图表数据..." standalone />
      )}
    </div>
  )
}
