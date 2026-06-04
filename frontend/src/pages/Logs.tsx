import { useState } from 'react'
import { formatDate, formatNumber, formatCost, formatLatency } from '@/lib/utils'
import { useLogs, useLogDetail, useLogModels, useChannels } from '@/api/query-hooks'
import { useAutoRefresh } from '@/hooks/useAutoRefresh'
import { ENDPOINT_LABELS } from '@/api/types'
import type { EndpointType, ChannelAttempt } from '@/api/types'
import { Button } from '@/components/ui/button'
import { Pagination } from '@/components/Pagination'
import { EmptyState } from '@/components/common'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  RefreshCw,
  CheckCircle2,
  XCircle,
  Copy,
  Check,
  Loader2,
  Clock,
  Zap,
  ArrowDownToLine,
  ArrowUpFromLine,
  DollarSign,
  Send,
  MessageSquare,
  AlertCircle,
} from 'lucide-react'

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false)
  const handleCopy = () => {
    navigator.clipboard.writeText(text)
    setCopied(true)
    setTimeout(() => setCopied(false), 2000)
  }
  return (
    <button onClick={handleCopy} className="shrink-0 p-1 rounded hover:bg-muted/80 text-muted-foreground hover:text-foreground transition-colors" title="复制">
      {copied ? <Check className="h-3.5 w-3.5 text-green-500" /> : <Copy className="h-3.5 w-3.5" />}
    </button>
  )
}

function tryParseJson(text: string): { isJson: boolean; data: unknown } {
  try {
    const parsed = JSON.parse(text)
    if (typeof parsed === 'object' && parsed !== null) {
      return { isJson: true, data: parsed }
    }
    return { isJson: false, data: text }
  } catch {
    return { isJson: false, data: text }
  }
}

function JsonBlock({ content, fallback }: { content: string | null; fallback: string }) {
  if (!content) {
    return <pre className="p-4 text-xs text-muted-foreground whitespace-pre-wrap">{fallback}</pre>
  }

  const { isJson, data } = tryParseJson(content)
  const displayText = isJson ? JSON.stringify(data, null, 2) : String(data)

  return (
    <div className="relative">
      <div className="sticky top-2 float-right mr-2 z-10">
        <CopyButton text={displayText} />
      </div>
      <pre className="p-4 text-xs font-mono whitespace-pre-wrap break-all leading-relaxed text-foreground/90">
        {displayText}
      </pre>
    </div>
  )
}

export function Logs() {
  const [page, setPage] = useState(1)
  const [pageSize, setPageSize] = useState(20)
  const [selectedModel, setSelectedModel] = useState('')
  const [selectedChannel, setSelectedChannel] = useState('')
  const [status, setStatus] = useState('')
  const [detailLogId, setDetailLogId] = useState<string | null>(null)

  const query = {
    page,
    page_size: pageSize,
    model: selectedModel || undefined,
    channel_id: selectedChannel || undefined,
    status: status || undefined,
  }

  const { data, isLoading, refetch } = useLogs(query)
  const { data: modelOptions = [] } = useLogModels()
  const { data: channelData } = useChannels()
  const { data: logDetail, isLoading: detailLoading } = useLogDetail(detailLogId)

  const { enabled: autoRefresh, toggle: toggleAutoRefresh } = useAutoRefresh({
    refetch: () => { refetch() },
    defaultInterval: 30,
    storageKey: 'logs-refresh',
    defaultEnabled: false,
  })

  const logs = data?.items ?? []
  const total = data?.total ?? 0
  const channelOptions = channelData?.items ?? []

  const openDetail = (logId: string) => {
    setDetailLogId(logId)
  }

  const closeDetail = () => {
    setDetailLogId(null)
  }

  const handleRefresh = () => {
    // Triggered by FilterBar button; React Query refetches on focus/window focus
    // We can force a refetch by toggling a key, but for simplicity we rely on staleTime
    window.dispatchEvent(new Event('focus'))
  }

  return (
    <div className="space-y-4">
      <div>
        <p className="text-sm text-muted-foreground">查看每次 API 请求的详细记录</p>
      </div>

      {/* 筛选栏 */}
      <div className="flex items-center gap-3 flex-wrap">
        <select
          value={selectedModel}
          onChange={(e) => { setSelectedModel(e.target.value); setPage(1) }}
          className="input w-48"
        >
          <option value="">全部模型</option>
          {modelOptions.map(m => (
            <option key={m} value={m}>{m}</option>
          ))}
        </select>
        <select
          value={selectedChannel}
          onChange={(e) => { setSelectedChannel(e.target.value); setPage(1) }}
          className="input w-36"
        >
          <option value="">全部渠道</option>
          {channelOptions.map(c => (
            <option key={c.id} value={c.id}>{c.name}</option>
          ))}
        </select>
        <select
          value={status}
          onChange={(e) => { setStatus(e.target.value); setPage(1) }}
          className="input w-28"
        >
          <option value="">全部状态</option>
          <option value="success">成功</option>
          <option value="failure">失败</option>
        </select>
        <Button variant="outline" size="icon" onClick={handleRefresh} title="刷新" disabled={isLoading}>
          {isLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
        </Button>
        <Button
          variant={autoRefresh ? 'default' : 'outline'}
          size="sm"
          onClick={toggleAutoRefresh}
        >
          自动刷新
        </Button>
      </div>

      {/* 表格 */}
      <div className="rounded-2xl border bg-card overflow-hidden">
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b bg-muted/50">
                <th className="text-left px-4 py-3 font-medium whitespace-nowrap">时间</th>
                <th className="text-left px-4 py-3 font-medium">模型</th>
                <th className="text-left px-4 py-3 font-medium">渠道</th>
                <th className="text-center px-4 py-3 font-medium">端点</th>
                <th className="text-center px-4 py-3 font-medium">类型</th>
                <th className="text-center px-4 py-3 font-medium">状态</th>
                <th className="text-right px-4 py-3 font-medium">输入</th>
                <th className="text-right px-4 py-3 font-medium">输出</th>
                <th className="text-right px-4 py-3 font-medium">耗时</th>
                <th className="text-right px-4 py-3 font-medium">TTFT</th>
                <th className="text-right px-4 py-3 font-medium">成本</th>
              </tr>
            </thead>
            <tbody>
              <EmptyState
                loading={isLoading}
                isEmpty={!isLoading && logs.length === 0}
                colSpan={11}
              />
              {!isLoading && logs.length > 0 && logs.map((log) => (
                <tr
                  key={log.id}
                  className="border-b last:border-0 hover:bg-muted/30 transition-colors cursor-pointer"
                  onClick={() => openDetail(log.id)}
                >
                  <td className="px-4 py-3 text-xs text-muted-foreground whitespace-nowrap">{formatDate(log.created_at)}</td>
                  <td className="px-4 py-3">
                    <div>
                      <p className="font-medium text-sm">{log.requested_model}</p>
                      {log.actual_model && log.actual_model !== log.requested_model && (
                        <p className="text-xs text-muted-foreground">→ {log.actual_model}</p>
                      )}
                    </div>
                  </td>
                  <td className="px-4 py-3 text-muted-foreground text-xs">{log.channel_name ?? '-'}</td>
                  <td className="px-4 py-3 text-center">
                    <span className="inline-flex items-center rounded-md bg-primary/10 px-1.5 py-0.5 text-xs font-medium text-primary">
                      {log.endpoint_type ? (ENDPOINT_LABELS[log.endpoint_type as EndpointType] ?? log.endpoint_type) : '-'}
                    </span>
                  </td>
                  <td className="px-4 py-3 text-center">
                    <span className={`inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium ${
                      log.request_type === 'passthrough'
                        ? 'bg-blue-100 text-blue-700 dark:bg-blue-900/30 dark:text-blue-400'
                        : 'bg-purple-100 text-purple-700 dark:bg-purple-900/30 dark:text-purple-400'
                    }`}>
                      {log.request_type === 'passthrough' ? '直通' : '转换'}
                    </span>
                    {log.is_stream && (
                      <span className="ml-1 inline-flex items-center rounded-full px-1.5 py-0.5 text-xs font-medium bg-amber-100 text-amber-700 dark:bg-amber-900/30 dark:text-amber-400">
                        流式
                      </span>
                    )}
                  </td>
                  <td className="px-4 py-3 text-center">
                    {log.error_message ? (
                      <span className="inline-flex items-center gap-1 text-destructive text-xs">
                        <XCircle className="h-3.5 w-3.5" />
                        {log.status_code ?? 'ERR'}
                      </span>
                    ) : (
                      <span className="inline-flex items-center gap-1 text-green-600 text-xs">
                        <CheckCircle2 className="h-3.5 w-3.5" />
                        {log.status_code ?? 200}
                      </span>
                    )}
                  </td>
                  <td className="px-4 py-3 text-right text-xs">{formatNumber(log.input_tokens)}</td>
                  <td className="px-4 py-3 text-right text-xs">{formatNumber(log.output_tokens)}</td>
                  <td className="px-4 py-3 text-right text-xs text-muted-foreground">
                    {formatLatency(log.latency_ms)}
                  </td>
                  <td className="px-4 py-3 text-right text-xs text-muted-foreground">
                    {log.ttft_ms != null ? formatLatency(log.ttft_ms) : '-'}
                  </td>
                  <td className="px-4 py-3 text-right text-xs">{formatCost(log.cost)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        <Pagination total={total} page={page} pageSize={pageSize} onPageChange={setPage} onPageSizeChange={setPageSize} pageSizeOptions={[20, 50, 100]} />
      </div>

      {/* 详情弹窗 */}
      <Dialog open={!!detailLogId} onOpenChange={(open) => { if (!open) closeDetail() }}>
        <DialogContent className="max-w-4xl h-[85vh] overflow-hidden flex flex-col p-0 gap-0">
          {detailLoading ? (
            <div className="flex items-center justify-center h-48">
              <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
              <span className="ml-2 text-sm text-muted-foreground">加载详情...</span>
            </div>
          ) : logDetail ? (
            <>
              <DialogHeader className="px-6 pt-6 pb-0">
                <DialogTitle className="flex items-center gap-3 text-base">
                  <span className="font-semibold">{logDetail.requested_model}</span>
                  {logDetail.actual_model && logDetail.actual_model !== logDetail.requested_model && (
                    <>
                      <span className="text-muted-foreground">→</span>
                      <span className="text-muted-foreground">{logDetail.actual_model}</span>
                    </>
                  )}
                  {logDetail.error_message ? (
                    <span className="ml-2 inline-flex items-center gap-1 text-xs text-destructive font-normal">
                      <XCircle className="h-3.5 w-3.5" />
                      {logDetail.status_code ?? 'ERR'}
                    </span>
                  ) : (
                    <span className="ml-2 inline-flex items-center gap-1 text-xs text-green-600 font-normal">
                      <CheckCircle2 className="h-3.5 w-3.5" />
                      {logDetail.status_code ?? 200}
                    </span>
                  )}
                </DialogTitle>
              </DialogHeader>

              {/* 指标条 */}
              <div className="flex flex-wrap items-center gap-x-5 gap-y-2 px-6 py-3 text-xs text-muted-foreground border-b">
                <div className="flex items-center gap-1.5">
                  <Clock className="h-3.5 w-3.5" />
                  <span className="tabular-nums">{formatDate(logDetail.created_at)}</span>
                </div>
                <div className="flex items-center gap-1.5">
                  <span>渠道: {logDetail.channel_name ?? '-'}</span>
                </div>
                <div className="flex items-center gap-1.5">
                  <span>Key: </span>
                  <span className="font-mono" title={logDetail.upstream_key_hint ?? undefined}>{logDetail.upstream_key_hint ?? '-'}</span>
                </div>
                <div className="flex items-center gap-1.5">
                  <Zap className="h-3.5 w-3.5 text-amber-500" />
                  <span>耗时 {formatLatency(logDetail.latency_ms)}</span>
                </div>
                {logDetail.ttft_ms != null && (
                  <div className="flex items-center gap-1.5">
                    <Zap className="h-3.5 w-3.5 text-orange-500" />
                    <span>TTFT {formatLatency(logDetail.ttft_ms)}</span>
                  </div>
                )}
                {logDetail.attempts && logDetail.attempts.length > 1 && (
                  <div className="flex items-center gap-1.5">
                    <RefreshCw className="h-3.5 w-3.5 text-muted-foreground" />
                    <span>{logDetail.attempts.length} 次尝试</span>
                  </div>
                )}
                <div className="flex items-center gap-1.5">
                  <ArrowDownToLine className="h-3.5 w-3.5 text-green-500" />
                  <span>输入 {formatNumber(logDetail.input_tokens)}</span>
                </div>
                <div className="flex items-center gap-1.5">
                  <ArrowUpFromLine className="h-3.5 w-3.5 text-purple-500" />
                  <span>输出 {formatNumber(logDetail.output_tokens)}</span>
                </div>
                <div className="flex items-center gap-1.5">
                  <DollarSign className="h-3.5 w-3.5 text-emerald-500" />
                  <span className="font-medium text-emerald-600 dark:text-emerald-400">{formatCost(logDetail.cost)}</span>
                </div>
                <div className="flex items-center gap-1.5">
                  <span className="inline-flex items-center rounded-md bg-primary/10 px-1.5 py-0.5 text-primary">
                    {logDetail.endpoint_type ? (ENDPOINT_LABELS[logDetail.endpoint_type as EndpointType] ?? logDetail.endpoint_type) : '-'}
                  </span>
                </div>
                <div className="flex items-center gap-1.5">
                  <span className={`inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium ${
                    logDetail.request_type === 'passthrough'
                      ? 'bg-blue-100 text-blue-700 dark:bg-blue-900/30 dark:text-blue-400'
                      : 'bg-purple-100 text-purple-700 dark:bg-purple-900/30 dark:text-purple-400'
                  }`}>
                    {logDetail.request_type === 'passthrough' ? '直通' : '转换'}
                  </span>
                  <span className={`inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium ${
                    logDetail.is_stream
                      ? 'bg-amber-100 text-amber-700 dark:bg-amber-900/30 dark:text-amber-400'
                      : 'bg-slate-100 text-slate-600 dark:bg-slate-800/30 dark:text-slate-400'
                  }`}>
                    {logDetail.is_stream ? '流式' : '非流式'}
                  </span>
                </div>
                {logDetail.user_agent && (
                  <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
                    <span className="font-mono truncate" title={logDetail.user_agent}>{logDetail.user_agent}</span>
                  </div>
                )}
              </div>

              {/* 错误信息 */}
              {logDetail.error_message && (
                <div className="mx-6 mt-4 p-3 rounded-xl bg-destructive/10 border border-destructive/20">
                  <div className="flex items-start gap-2">
                    <AlertCircle className="h-4 w-4 shrink-0 mt-0.5 text-destructive" />
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2 mb-1">
                        <span className="text-sm font-medium text-destructive">错误信息</span>
                        <CopyButton text={logDetail.error_message} />
                      </div>
                      <pre className="text-xs text-destructive whitespace-pre-wrap break-all leading-relaxed">{logDetail.error_message}</pre>
                    </div>
                  </div>
                </div>
              )}

              {/* 重试链路 */}
              {logDetail.attempts && logDetail.attempts.length > 0 && (
                <div className="mx-6 mt-4">
                  <div className="flex items-center gap-2 mb-2">
                    <span className="text-sm font-medium">重试链路</span>
                    <span className="text-xs text-muted-foreground">({logDetail.attempts.length} 次尝试)</span>
                  </div>
                  <div className="space-y-1">
                    {logDetail.attempts.map((a: ChannelAttempt, i: number) => (
                      <div key={i} className="flex items-center gap-3 text-xs px-3 py-2 rounded-lg bg-muted/50">
                        <span className="text-muted-foreground w-4 text-right">#{i + 1}</span>
                        <span className="font-mono text-muted-foreground">{a.channel_id.slice(0, 8)}</span>
                        {a.upstream_key_hint && (
                          <span className="font-mono text-muted-foreground/70" title={a.upstream_key_hint}>{a.upstream_key_hint.length > 16 ? a.upstream_key_hint.slice(0, 16) + '…' : a.upstream_key_hint}</span>
                        )}
                        <span className={`inline-flex items-center gap-1 rounded-full px-1.5 py-0.5 text-xs font-medium ${
                          a.status === 'success'
                            ? 'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400'
                            : 'bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400'
                        }`}>
                          {a.status === 'success' ? <CheckCircle2 className="h-3 w-3" /> : <XCircle className="h-3 w-3" />}
                          {a.status}
                        </span>
                        <span className="text-muted-foreground">{formatLatency(a.duration_ms)}</span>
                        {a.error && (
                          <span className="text-destructive truncate max-w-[300px]" title={a.error}>{a.error}</span>
                        )}
                      </div>
                    ))}
                  </div>
                </div>
              )}

              {/* 请求/响应内容 */}
              <div className="flex-1 min-h-0 px-6 py-4">
                <div className="grid grid-cols-1 md:grid-cols-2 gap-4 h-full min-h-0">
                  <div className="flex flex-col rounded-xl border bg-muted/30 min-h-0 overflow-hidden">
                    <div className="flex items-center gap-2 px-3 py-2.5 border-b bg-muted/50 shrink-0">
                      <Send className="h-4 w-4 text-green-500" />
                      <span className="text-sm font-medium">请求内容</span>
                      <span className="ml-auto text-xs text-muted-foreground">{formatNumber(logDetail.input_tokens)} tokens</span>
                    </div>
                    <div className="flex-1 min-h-0 overflow-auto">
                      <JsonBlock content={logDetail.request_content} fallback="无请求内容" />
                    </div>
                  </div>
                  <div className="flex flex-col rounded-xl border bg-muted/30 min-h-0 overflow-hidden">
                    <div className="flex items-center gap-2 px-3 py-2.5 border-b bg-muted/50 shrink-0">
                      <MessageSquare className="h-4 w-4 text-purple-500" />
                      <span className="text-sm font-medium">响应内容</span>
                      <span className="ml-auto text-xs text-muted-foreground">{formatNumber(logDetail.output_tokens)} tokens</span>
                    </div>
                    <div className="flex-1 min-h-0 overflow-auto">
                      <JsonBlock content={logDetail.response_content} fallback="无响应内容" />
                    </div>
                  </div>
                </div>
              </div>
            </>
          ) : null}
        </DialogContent>
      </Dialog>
    </div>
  )
}
