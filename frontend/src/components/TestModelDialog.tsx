import { Fragment, useCallback, useMemo, useState } from 'react'
import type { Channel, EndpointType, TestEndpointResponse } from '@/api/types'
import { ENDPOINT_LABELS } from '@/api/types'
import { channelsApi } from '@/api/channels'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { StatusBadge } from '@/components/StatusBadge'
import { Play, Loader2, Search, ChevronRight } from 'lucide-react'

interface TestModelDialogProps {
  channel: Channel | null
  open: boolean
  onOpenChange: (open: boolean) => void
}

type ModelStatus = 'idle' | 'testing' | 'success' | 'error'

interface ModelTestResult {
  status: ModelStatus
  latency_ms?: number
  time_to_first_token_ms?: number
  error?: string
  output_content?: string
  reasoning?: string
  prompt_tokens?: number
  completion_tokens?: number
  thinking_detected?: boolean
}

function maskKey(key: string) {
  if (key.length <= 8) return '****'
  return '...' + key.slice(-4)
}

function keyLabel(k: { key: string; note?: string }) {
  return k.note?.trim() || maskKey(k.key)
}

function TerminalOutput({
  result,
  isTesting,
  protocolLabel,
}: {
  result?: ModelTestResult
  isTesting: boolean
  protocolLabel: string
}) {
  return (
    <div className="rounded-lg border border-gray-700 bg-gray-900 dark:bg-black p-3 font-mono text-xs space-y-0.5 max-h-[200px] overflow-y-auto">
      {isTesting && (
        <>
          <div className="text-yellow-400 animate-pulse">▸ 测试 {protocolLabel} 协议...</div>
          <div className="text-yellow-400 animate-pulse">▌</div>
        </>
      )}
      {!isTesting && result?.status === 'success' && (
        <>
          <div className="text-green-400">✓ 渠道连接成功</div>
          <div className="text-cyan-400">→ 协议: {protocolLabel}</div>
          <div className="text-cyan-400">
            → 耗时: {result.latency_ms}ms
            {result.time_to_first_token_ms != null && `  TTFT: ${result.time_to_first_token_ms}ms`}
          </div>
          {(result.prompt_tokens != null || result.completion_tokens != null) && (
            <div className="text-cyan-400">
              → Token: {result.prompt_tokens ?? '-'}/{result.completion_tokens ?? '-'}
            </div>
          )}
          {result.reasoning && (
            <>
              <div className="text-gray-500">── 思维链 ──</div>
              <div className="text-yellow-200/80 whitespace-pre-wrap">{result.reasoning}</div>
            </>
          )}
          {result.thinking_detected && (
            <div className="text-yellow-300">⚡ 检测到思维链（&lt;think&gt; 标签）</div>
          )}
          {result.output_content && (
            <>
              <div className="text-gray-500">── 输出 ──</div>
              <div className="text-green-300 whitespace-pre-wrap">{result.output_content}</div>
            </>
          )}
          <div className="text-green-400">✓ 测试成功</div>
        </>
      )}
      {!isTesting && result?.status === 'error' && (
        <div className="text-red-400 whitespace-pre-wrap">✗ 测试失败: {result.error || '未知错误'}</div>
      )}
    </div>
  )
}

export function TestModelDialog({ channel, open, onOpenChange }: TestModelDialogProps) {
  const [selectedKey, setSelectedKey] = useState('')
  const [protocol, setProtocol] = useState('')
  const [searchTerm, setSearchTerm] = useState('')
  const [results, setResults] = useState<Record<string, ModelTestResult>>({})
  const [testingModels, setTestingModels] = useState<Set<string>>(new Set())
  const [isBatchTesting, setIsBatchTesting] = useState(false)
  const [expandedModels, setExpandedModels] = useState<Set<string>>(new Set())
  const [uaPreset, setUaPreset] = useState('')
  const [customUa, setCustomUa] = useState('')

  const effectiveUserAgent = uaPreset === 'custom' ? customUa.trim() : uaPreset

  const resetState = useCallback(() => {
    setSelectedKey('')
    setProtocol('')
    setSearchTerm('')
    setResults({})
    setTestingModels(new Set())
    setIsBatchTesting(false)
    setExpandedModels(new Set())
    setUaPreset('')
    setCustomUa('')
  }, [])

  const models = useMemo(() => channel?.models || [], [channel?.models])
  const enabledKeys = useMemo(
    () => channel?.api_keys.filter((k) => k.enabled !== false) || [],
    [channel?.api_keys]
  )
  const endpointTypes = new Set(
    (channel?.endpoints || []).filter((e) => e.enabled !== false).map((e) => e.type)
  )
  const availableProtocols = Object.entries(ENDPOINT_LABELS)
    .filter(([key]) => endpointTypes.has(key as EndpointType))
    .map(([key, label]) => ({ value: key as EndpointType, label }))

  const currentKey = selectedKey || enabledKeys[0]?.key || ''
  const currentProtocol = protocol || availableProtocols[0]?.value || ''
  const protocolLabel =
    availableProtocols.find((p) => p.value === currentProtocol)?.label || currentProtocol

  const filteredModels = useMemo(() => {
    if (!searchTerm) return models
    const kw = searchTerm.toLowerCase()
    return models.filter((m) => m.toLowerCase().includes(kw))
  }, [models, searchTerm])

  const updateResult = (model: string, result: ModelTestResult) => {
    setResults((prev) => ({ ...prev, [model]: result }))
  }

  const markTesting = (model: string, testing: boolean) => {
    setTestingModels((prev) => {
      const next = new Set(prev)
      if (testing) next.add(model)
      else next.delete(model)
      return next
    })
  }

  const toggleExpand = (model: string) => {
    setExpandedModels((prev) => {
      const next = new Set(prev)
      if (next.has(model)) next.delete(model)
      else next.add(model)
      return next
    })
  }

  const testSingle = async (model: string): Promise<ModelTestResult | undefined> => {
    if (!channel || !currentKey || !currentProtocol) return undefined

    markTesting(model, true)
    updateResult(model, { status: 'testing' })

    // 单测时自动展开终端
    setExpandedModels((prev) => {
      if (prev.has(model)) return prev
      const next = new Set(prev)
      next.add(model)
      return next
    })

    let finalResult: ModelTestResult | undefined
    try {
      // 找当前协议对应的 endpoint 配置（base_url + headers）；test-endpoint 不依赖 channel.id
      const ep = channel.endpoints.find((e) => e.type === currentProtocol && e.enabled !== false)
      const res: TestEndpointResponse = await channelsApi.testEndpoint({
        endpoint_type: currentProtocol,
        base_url: ep?.base_url ?? '',
        api_key: currentKey,
        model,
        headers: ep?.headers,
        user_agent: effectiveUserAgent || undefined,
      })
      finalResult = {
        status: res.success ? 'success' : 'error',
        latency_ms: res.latency_ms,
        time_to_first_token_ms: res.time_to_first_token_ms,
        error: res.success ? undefined : res.error,
        output_content: res.output_content ?? undefined,
        reasoning: res.reasoning,
        prompt_tokens: res.prompt_tokens,
        completion_tokens: res.completion_tokens,
        thinking_detected: res.thinking_detected,
      }
    } catch (e: unknown) {
      finalResult = {
        status: 'error',
        error: e instanceof Error ? e.message : '请求失败',
      }
    }

    updateResult(model, finalResult)
    markTesting(model, false)
    return finalResult
  }

  const handleBatchTest = async () => {
    const targets = filteredModels.length > 0 ? filteredModels : models
    if (targets.length === 0) return

    setIsBatchTesting(true)
    await Promise.allSettled(targets.map((m) => testSingle(m)))
    setIsBatchTesting(false)
  }

  const handleOpenChange = (v: boolean) => {
    if (!v) resetState()
    onOpenChange(v)
  }

  const isAnyTesting = testingModels.size > 0 || isBatchTesting
  const successCount = Object.values(results).filter((r) => r.status === 'success').length
  const failCount = Object.values(results).filter((r) => r.status === 'error').length

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="sm:max-w-3xl max-h-[90vh] overflow-hidden flex flex-col">
        <DialogHeader>
          <DialogTitle>渠道测试</DialogTitle>
          {channel && (
            <DialogDescription>
              测试 <strong>{channel.name}</strong> 的模型连通性
            </DialogDescription>
          )}
        </DialogHeader>

        <div className="flex-1 overflow-y-auto space-y-4 pr-1">
          {/* 渠道信息卡 */}
          {channel && (
            <div className="flex items-center justify-between rounded-xl bg-gradient-to-r from-primary/10 to-primary/5 border border-primary/10 px-4 py-3">
              <div className="flex items-center gap-3">
                <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-gradient-to-br from-primary to-primary/70 text-primary-foreground text-xs font-bold">
                  {channel.name.charAt(0)}
                </div>
                <div>
                  <span className="font-medium text-sm">{channel.name}</span>
                  <span className="text-xs text-muted-foreground ml-2">
                    {models.length} 模型 · {enabledKeys.length} Key · {availableProtocols.length} 协议
                  </span>
                </div>
              </div>
              <span
                className={`inline-flex items-center rounded-full px-2.5 py-0.5 text-xs font-medium ${
                  channel.enabled
                    ? 'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400'
                    : 'bg-gray-100 text-gray-500 dark:bg-gray-800 dark:text-gray-400'
                }`}
              >
                {channel.enabled ? '启用' : '禁用'}
              </span>
            </div>
          )}

          {/* 配置区 */}
          <div className="grid gap-4 sm:grid-cols-4">
            <div className="space-y-1.5">
              <label className="text-sm font-medium">API Key</label>
              <select
                value={currentKey}
                onChange={(e) => setSelectedKey(e.target.value)}
                className="input"
              >
                {enabledKeys.map((k) => (
                  <option key={k.key} value={k.key}>
                    {keyLabel(k)} {maskKey(k.key)}
                  </option>
                ))}
              </select>
            </div>

            <div className="space-y-1.5">
              <label className="text-sm font-medium">端点协议</label>
              <select
                value={currentProtocol}
                onChange={(e) => setProtocol(e.target.value)}
                className="input"
              >
                {availableProtocols.map((p) => (
                  <option key={p.value} value={p.value}>
                    {p.label}
                  </option>
                ))}
              </select>
            </div>

            <div className="space-y-1.5">
              <label className="text-sm font-medium">User-Agent</label>
              <select
                value={uaPreset}
                onChange={(e) => setUaPreset(e.target.value)}
                className="input"
              >
                <option value="">默认（不设置）</option>
                <option value="custom">自定义...</option>
                <option value="HermesAgent/0.14.0">HermesAgent/0.14.0</option>
                <option value="claude-cli/2.1.140 (external, cli)">claude-code</option>
              </select>
              {uaPreset === 'custom' && (
                <input
                  type="text"
                  value={customUa}
                  onChange={(e) => setCustomUa(e.target.value)}
                  placeholder="输入自定义 User-Agent..."
                  className="input text-xs"
                  autoFocus
                />
              )}
            </div>
          </div>

          {/* 搜索 + 批量操作 */}
          <div className="flex items-center justify-between gap-3">
            <span className="text-sm font-medium">
              模型 ({models.length})
              {successCount > 0 && <span className="text-green-600 ml-2">✓ {successCount}</span>}
              {failCount > 0 && <span className="text-red-500 ml-1">✗ {failCount}</span>}
            </span>
            <div className="flex items-center gap-2">
              <div className="relative">
                <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground" />
                <input
                  type="text"
                  value={searchTerm}
                  onChange={(e) => setSearchTerm(e.target.value)}
                  placeholder="过滤模型..."
                  className="input pl-8 w-44 text-sm"
                />
              </div>
              <Button
                size="sm"
                onClick={handleBatchTest}
                disabled={isAnyTesting || !currentKey || !currentProtocol || filteredModels.length === 0}
              >
                {isBatchTesting ? (
                  <>
                    <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                    测试中...
                  </>
                ) : (
                  <>
                    <Play className="mr-1.5 h-3.5 w-3.5" />
                    测试全部 ({filteredModels.length})
                  </>
                )}
              </Button>
            </div>
          </div>

          {/* 模型列表 */}
          <div className="rounded-xl border overflow-hidden">
            <div className="max-h-[400px] overflow-y-auto">
              <table className="w-full text-sm">
                <thead className="sticky top-0 bg-muted/80 backdrop-blur-sm z-10">
                  <tr className="border-b">
                    <th className="text-left px-3 py-2 font-medium">模型</th>
                    <th className="text-left px-3 py-2 font-medium w-60">状态</th>
                    <th className="text-right px-3 py-2 font-medium w-20">操作</th>
                  </tr>
                </thead>
                <tbody>
                  {filteredModels.map((model) => {
                    const r = results[model]
                    const isTesting = testingModels.has(model)
                    const isExpanded = expandedModels.has(model)
                    const hasDetail = r && r.status !== 'idle'

                    return (
                      <Fragment key={model}>
                        <tr className="border-b last:border-0 hover:bg-muted/30">
                          <td className="px-3 py-2">
                            <div className="flex items-center gap-1.5">
                              {hasDetail ? (
                                <button
                                  onClick={() => toggleExpand(model)}
                                  className="p-0.5 hover:bg-muted rounded"
                                >
                                  <ChevronRight
                                    className={`h-3.5 w-3.5 text-muted-foreground transition-transform ${isExpanded ? 'rotate-90' : ''}`}
                                  />
                                </button>
                              ) : (
                                <span className="w-[18px] inline-block" />
                              )}
                              <span className="font-mono text-xs">{model}</span>
                            </div>
                          </td>
                          <td className="px-3 py-2">
                            {!r || r.status === 'idle' ? (
                              <span className="text-muted-foreground text-xs">未测试</span>
                            ) : r.status === 'testing' ? (
                              <div className="flex items-center gap-1.5 text-muted-foreground text-xs">
                                <Loader2 className="h-3 w-3 animate-spin" />
                                测试中...
                              </div>
                            ) : r.status === 'success' ? (
                              <div className="flex items-center gap-2 text-xs">
                                <StatusBadge enabled onClick={() => {}} />
                                <span className="text-muted-foreground">
                                  {r.latency_ms}ms
                                  {r.time_to_first_token_ms != null && (
                                    <span className="ml-1">TTFT {r.time_to_first_token_ms}ms</span>
                                  )}
                                </span>
                                {(r.prompt_tokens != null || r.completion_tokens != null) && (
                                  <span className="text-muted-foreground">
                                    {r.prompt_tokens ?? '-'}/{r.completion_tokens ?? '-'}
                                  </span>
                                )}
                              </div>
                            ) : (
                              <div className="flex items-center gap-1 text-xs">
                                <StatusBadge enabled={false} onClick={() => {}} />
                                <span className="text-red-500 truncate max-w-[180px]" title={r.error}>
                                  {r.error || '测试失败'}
                                </span>
                              </div>
                            )}
                          </td>
                          <td className="px-3 py-2 text-right">
                            <Button
                              variant="ghost"
                              size="sm"
                              className="h-7 text-xs"
                              onClick={() => testSingle(model)}
                              disabled={isTesting || isBatchTesting || !currentKey || !currentProtocol}
                            >
                              {isTesting ? <Loader2 className="h-3 w-3 animate-spin" /> : '测试'}
                            </Button>
                          </td>
                        </tr>
                        {isExpanded && hasDetail && (
                          <tr className="border-b">
                            <td colSpan={3} className="p-0">
                              <div className="px-3 py-2">
                                <TerminalOutput
                                  result={r}
                                  isTesting={isTesting}
                                  protocolLabel={protocolLabel}
                                />
                              </div>
                            </td>
                          </tr>
                        )}
                      </Fragment>
                    )
                  })}
                  {filteredModels.length === 0 && (
                    <tr>
                      <td colSpan={3} className="text-center py-8 text-muted-foreground text-xs">
                        {models.length === 0 ? '该渠道没有配置模型' : '没有匹配的模型'}
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}
