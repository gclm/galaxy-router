import { useCallback, useMemo, useState } from 'react'
import type { Channel, EndpointType, TestChannelResponse } from '@/api/types'
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
import { Play, Loader2, Search } from 'lucide-react'

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
}

function maskKey(key: string) {
  if (key.length <= 8) return '****'
  return '...' + key.slice(-4)
}

export function TestModelDialog({ channel, open, onOpenChange }: TestModelDialogProps) {
  const [selectedKeyId, setSelectedKeyId] = useState('')
  const [protocol, setProtocol] = useState('')
  const [useStream, setUseStream] = useState(false)
  const [searchTerm, setSearchTerm] = useState('')
  const [results, setResults] = useState<Record<string, ModelTestResult>>({})
  const [testingModels, setTestingModels] = useState<Set<string>>(new Set())
  const [isBatchTesting, setIsBatchTesting] = useState(false)

  const resetState = useCallback(() => {
    setSelectedKeyId('')
    setProtocol('')
    setUseStream(false)
    setSearchTerm('')
    setResults({})
    setTestingModels(new Set())
    setIsBatchTesting(false)
  }, [])

  if (!channel) return null

  const models = channel.models || []
  const enabledKeys = channel.api_keys.filter((k) => k.enabled !== false)
  const endpointTypes = new Set(
    channel.endpoints.filter((e) => e.enabled !== false).map((e) => e.type)
  )
  const availableProtocols = Object.entries(ENDPOINT_LABELS)
    .filter(([key]) => endpointTypes.has(key as EndpointType))
    .map(([key, label]) => ({ value: key as EndpointType, label }))

  const currentKeyId = selectedKeyId || enabledKeys[0]?.id || ''
  const currentProtocol = protocol || availableProtocols[0]?.value || ''

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

  const testSingle = async (model: string): Promise<ModelTestResult | undefined> => {
    if (!currentKeyId || !currentProtocol) return undefined

    markTesting(model, true)
    updateResult(model, { status: 'testing' })

    let finalResult: ModelTestResult | undefined
    try {
      const res: TestChannelResponse = await channelsApi.testChannel(channel.id, {
        model,
        test_protocol: currentProtocol,
        api_key_id: currentKeyId,
        stream: useStream || undefined,
      })
      finalResult = {
        status: res.success ? 'success' : 'error',
        latency_ms: res.latency_ms,
        time_to_first_token_ms: res.time_to_first_token_ms,
        error: res.success ? undefined : res.message,
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
          <DialogDescription>
            测试 <strong>{channel.name}</strong> 的模型连通性
          </DialogDescription>
        </DialogHeader>

        <div className="flex-1 overflow-y-auto space-y-4 pr-1">
          {/* 配置区 */}
          <div className="grid gap-4 sm:grid-cols-3">
            {/* Key 选择 */}
            <div className="space-y-1.5">
              <label className="text-sm font-medium">API Key</label>
              <select
                value={currentKeyId}
                onChange={(e) => setSelectedKeyId(e.target.value)}
                className="input"
              >
                {enabledKeys.map((k) => (
                  <option key={k.id} value={k.id}>
                    {k.note || '未命名'} {maskKey(k.key)}
                  </option>
                ))}
              </select>
            </div>

            {/* 协议选择 */}
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

            {/* 流式开关 */}
            <div className="space-y-1.5">
              <label className="text-sm font-medium">流式测试</label>
              <div className="flex items-center gap-2 h-[38px]">
                <input
                  type="checkbox"
                  checked={useStream}
                  onChange={(e) => setUseStream(e.target.checked)}
                  className="rounded"
                />
                <span className="text-sm text-muted-foreground">
                  {useStream ? '开启' : '关闭'}
                </span>
              </div>
            </div>
          </div>

          {/* 搜索 + 批量操作 */}
          <div className="flex items-center justify-between gap-3">
            <div className="flex-1 flex items-center gap-2">
              <span className="text-sm font-medium">
                模型 ({models.length})
                {successCount > 0 && (
                  <span className="text-green-600 ml-2">✓ {successCount}</span>
                )}
                {failCount > 0 && (
                  <span className="text-red-500 ml-1">✗ {failCount}</span>
                )}
              </span>
            </div>
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
                disabled={isAnyTesting || !currentKeyId || !currentProtocol || filteredModels.length === 0}
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
                <thead className="sticky top-0 bg-muted/80 backdrop-blur-sm">
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
                    return (
                      <tr key={model} className="border-b last:border-0 hover:bg-muted/30">
                        <td className="px-3 py-2 font-mono text-xs">{model}</td>
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
                            disabled={isTesting || isBatchTesting || !currentKeyId || !currentProtocol}
                          >
                            {isTesting ? (
                              <Loader2 className="h-3 w-3 animate-spin" />
                            ) : (
                              '测试'
                            )}
                          </Button>
                        </td>
                      </tr>
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
