import { useState } from 'react'
import type { Channel, CreateChannelRequest, CustomHeader, EndpointConfig, EndpointType, TestEndpointResponse, UpstreamApiKey } from '@/api/types'
import { ENDPOINT_LABELS } from '@/api/types'
import { channelsApi } from '@/api/channels'
import { Button } from '@/components/ui/button'
import { Plus, Trash2, RefreshCw, X, FlaskConical } from 'lucide-react'

interface ChannelFormProps {
  channel?: Channel
  onSubmit: (data: CreateChannelRequest) => Promise<void>
  onCancel: () => void
}

const ENDPOINT_TYPES: EndpointType[] = [
  'openai_chat',
  'openai_response',
  'anthropic',
  'gemini',
  'openai_embedding',
  'openai_images',
]

/** 常见 coding agent User-Agent 模板（选模板快捷填入，也可自定义任意 header） */
const UA_TEMPLATES: { label: string; value: string }[] = [
  { label: 'Claude Code', value: 'claude-code/2.1.0 cli' },
  { label: 'Cline', value: 'cline/1.0.0' },
  { label: 'Cursor', value: 'cursor/0.42.0' },
  { label: 'Roo Code', value: 'roo-cline/1.0.0' },
  { label: 'Continue', value: 'continue/0.9.0' },
]

export function ChannelForm({ channel, onSubmit, onCancel }: ChannelFormProps) {
  const [name, setName] = useState(channel?.name ?? '')
  const [apiKeys, setApiKeys] = useState<UpstreamApiKey[]>(
    channel?.api_keys?.map(k => typeof k === 'string' ? { key: k, note: '', enabled: true } : { key: k.key, note: k.note ?? '', enabled: k.enabled ?? true }) ?? [{ key: '', note: '', enabled: true }]
  )
  const [endpoints, setEndpoints] = useState<EndpointConfig[]>(
    channel?.endpoints ?? [{ type: 'openai_chat', base_url: '', enabled: true, headers: [] }]
  )
  const [models, setModels] = useState<string[]>(channel?.models ?? [])
  const [rateLimitRpm, setRateLimitRpm] = useState(channel?.rate_limit_rpm?.toString() ?? '')
  const [rateLimitTpm, setRateLimitTpm] = useState(channel?.rate_limit_tpm?.toString() ?? '')
  const [failureThreshold, setFailureThreshold] = useState(channel?.failure_threshold?.toString() ?? '3')
  const [blacklistMinutes, setBlacklistMinutes] = useState(channel?.blacklist_minutes?.toString() ?? '5')
  const [concurrency, setConcurrency] = useState(channel?.concurrency?.toString() ?? '10')
  const [timeoutSecs, setTimeoutSecs] = useState(channel?.timeout_secs?.toString() ?? '300')
  const [maxConcurrency, setMaxConcurrency] = useState(channel?.max_concurrency?.toString() ?? '0')
  const [enabled, setEnabled] = useState(channel?.enabled ?? true)
  const [submitting, setSubmitting] = useState(false)
  const [fetchingModels, setFetchingModels] = useState(false)
  const [fetchError, setFetchError] = useState('')
  const [manualModelInput, setManualModelInput] = useState('')
  const [endpointTesting, setEndpointTesting] = useState<Record<number, boolean>>({})
  const [endpointResults, setEndpointResults] = useState<Record<number, TestEndpointResponse>>({})

  const handleFetchModels = async () => {
    const validEndpoints = endpoints.filter(ep => ep.base_url.trim() && ep.enabled !== false)
    const apiKey = apiKeys.find(k => k.key.trim() && k.enabled !== false)?.key

    if (validEndpoints.length === 0 || !apiKey) {
      alert('请先填写端点地址和 API Key')
      return
    }

    setFetchingModels(true)
    setFetchError('')
    try {
      const fetched = await channelsApi.fetchModels({
        endpoints: validEndpoints,
        api_key: apiKey,
      })
      setModels(fetched)
    } catch (e: unknown) {
      setFetchError(e instanceof Error ? e.message : '获取模型失败')
    } finally {
      setFetchingModels(false)
    }
  }

  const addManualModel = () => {
    const model = manualModelInput.trim()
    if (model && !models.includes(model)) {
      setModels(prev => [...prev, model])
      setManualModelInput('')
    }
  }

  const removeModel = (model: string) => {
    setModels(prev => prev.filter(m => m !== model))
  }

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    setSubmitting(true)

    try {
      const data: CreateChannelRequest = {
        name,
        api_keys: apiKeys.filter((k) => k.key.trim()).map(k => ({ key: k.key, note: k.note, enabled: k.enabled })),
        endpoints: endpoints.filter((ep) => ep.base_url.trim()).map(ep => ({ type: ep.type, base_url: ep.base_url, enabled: ep.enabled, headers: (ep.headers ?? []).filter(h => h.key.trim()) })),
        models,
        enabled,
        failure_threshold: parseInt(failureThreshold) || 3,
        blacklist_minutes: parseInt(blacklistMinutes) || 5,
        concurrency: parseInt(concurrency) || 10,
        timeout_secs: parseInt(timeoutSecs) || 300,
        max_concurrency: parseInt(maxConcurrency) || 0,
      }

      if (rateLimitRpm) data.rate_limit_rpm = parseInt(rateLimitRpm)
      if (rateLimitTpm) data.rate_limit_tpm = parseInt(rateLimitTpm)

      await onSubmit(data)
    } finally {
      setSubmitting(false)
    }
  }

  const addApiKey = () => setApiKeys([...apiKeys, { key: '', note: '', enabled: true }])
  const removeApiKey = (index: number) => setApiKeys(apiKeys.filter((_, i) => i !== index))
  const updateApiKey = (index: number, field: keyof UpstreamApiKey, value: string | boolean) => {
    const newKeys = [...apiKeys]
    newKeys[index] = { ...newKeys[index], [field]: value }
    setApiKeys(newKeys)
  }

  const addEndpoint = () => setEndpoints([...endpoints, { type: 'openai_chat', base_url: '', enabled: true, headers: [] }])
  const removeEndpoint = (index: number) => setEndpoints(endpoints.filter((_, i) => i !== index))
  const updateEndpoint = (index: number, field: keyof EndpointConfig, value: string | boolean) => {
    const newEndpoints = [...endpoints]
    newEndpoints[index] = { ...newEndpoints[index], [field]: value }
    setEndpoints(newEndpoints)
  }
  // 端点级 headers 操作
  const setEndpointHeaders = (index: number, headers: CustomHeader[]) => {
    const newEndpoints = [...endpoints]
    newEndpoints[index] = { ...newEndpoints[index], headers }
    setEndpoints(newEndpoints)
  }
  const addEndpointHeader = (index: number) =>
    setEndpointHeaders(index, [...(endpoints[index].headers ?? []), { key: '', value: '' }])
  const updateEndpointHeader = (index: number, hi: number, field: 'key' | 'value', value: string) => {
    const headers = [...(endpoints[index].headers ?? [])]
    headers[hi] = { ...headers[hi], [field]: value }
    setEndpointHeaders(index, headers)
  }
  const removeEndpointHeader = (index: number, hi: number) =>
    setEndpointHeaders(index, (endpoints[index].headers ?? []).filter((_, j) => j !== hi))
  /** 选 UA 模板：写入/覆盖该端点的 User-Agent header */
  const applyUaTemplate = (index: number, ua: string) => {
    if (!ua) return
    const headers = [...(endpoints[index].headers ?? [])]
    const idx = headers.findIndex((h) => h.key.toLowerCase() === 'user-agent')
    const entry = { key: 'User-Agent', value: ua }
    if (idx >= 0) headers[idx] = entry
    else headers.push(entry)
    setEndpointHeaders(index, headers)
  }

  /** 端点测试：调 testEndpoint 探测连通性 + 思维链诊断（结果仅展示，不驱动配置） */
  const testEndpoint = async (index: number) => {
    if (!channel?.id) {
      alert('请先保存渠道后再测试')
      return
    }
    const ep = endpoints[index]
    const apiKey = apiKeys.find((k) => k.enabled !== false)?.key
    const model = models[0]
    if (!apiKey) {
      alert('没有可用的 API Key')
      return
    }
    if (!model) {
      alert('请先添加模型')
      return
    }
    setEndpointTesting((prev) => ({ ...prev, [index]: true }))
    try {
      const res = await channelsApi.testEndpoint(channel.id, {
        endpoint_type: ep.type,
        model,
        api_key: apiKey,
      })
      setEndpointResults((prev) => ({ ...prev, [index]: res }))
    } catch (e: unknown) {
      setEndpointResults((prev) => ({
        ...prev,
        [index]: {
          success: false,
          latency_ms: 0,
          error: e instanceof Error ? e.message : '测试失败',
          thinking_detected: false,
        },
      }))
    } finally {
      setEndpointTesting((prev) => ({ ...prev, [index]: false }))
    }
  }
  return (
    <form onSubmit={handleSubmit} className="space-y-5 px-1">
      {/* 基本信息 */}
      <section className="space-y-3">
        <h3 className="text-sm font-medium text-muted-foreground">基本信息</h3>
        <div>
          <label className="block text-sm font-medium mb-1">渠道名称 *</label>
          <input
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            className="input"
            placeholder="例如：OpenAI 主力渠道"
            required
          />
        </div>
        <label className="flex items-center gap-2 text-sm">
          <input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} className="rounded" />
          启用渠道
        </label>
      </section>

      {/* 端点配置 */}
      <section className="space-y-3">
        <div className="flex items-center justify-between">
          <div>
            <h3 className="text-sm font-medium text-muted-foreground">上游端点配置 *</h3>
            <p className="text-xs text-muted-foreground mt-1">Gemini 仅用于管理侧模型探测，不属于正式代理协议。</p>
          </div>
          <Button type="button" variant="outline" size="sm" onClick={addEndpoint}>
            <Plus className="h-4 w-4 mr-1" /> 添加
          </Button>
        </div>
        {endpoints.map((ep, index) => (
          <div key={index} className="space-y-3 rounded-xl border bg-muted/20 p-4">
            <div className="flex gap-2 items-center">
              <select
                value={ep.type}
                onChange={(e) => updateEndpoint(index, 'type', e.target.value)}
                className="input w-44"
              >
                {ENDPOINT_TYPES.map((t) => (
                  <option key={t} value={t}>{ENDPOINT_LABELS[t]}</option>
                ))}
              </select>
              <input
                type="text"
                value={ep.base_url}
                onChange={(e) => updateEndpoint(index, 'base_url', e.target.value)}
                className="input flex-1"
                placeholder="https://api.openai.com/v1"
              />
              <label className="flex items-center gap-1 text-xs text-muted-foreground whitespace-nowrap cursor-pointer">
                <input
                  type="checkbox"
                  checked={ep.enabled !== false}
                  onChange={(e) => updateEndpoint(index, 'enabled', e.target.checked)}
                  className="rounded"
                />
                启用
              </label>
              {channel && (
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  onClick={() => testEndpoint(index)}
                  disabled={endpointTesting[index]}
                  title="测试端点"
                >
                  <FlaskConical className={`h-4 w-4 ${endpointTesting[index] ? 'animate-spin' : ''}`} />
                </Button>
              )}
              {endpoints.length > 1 && (
                <Button type="button" variant="ghost" size="icon" onClick={() => removeEndpoint(index)}>
                  <Trash2 className="h-4 w-4" />
                </Button>
              )}
            </div>
            <div className="space-y-2 pl-1">
              <div className="flex items-center gap-2">
                <span className="w-12 shrink-0 text-xs font-medium text-muted-foreground">请求头</span>
                <select
                  value=""
                  onChange={(e) => applyUaTemplate(index, e.target.value)}
                  className="input w-48 text-sm"
                >
                  <option value="">选择 UA 模板...</option>
                  {UA_TEMPLATES.map((t) => (
                    <option key={t.value} value={t.value}>{t.label}</option>
                  ))}
                </select>
                <Button type="button" variant="outline" size="sm" onClick={() => addEndpointHeader(index)}>
                  <Plus className="h-4 w-4 mr-1" /> 自定义 Header
                </Button>
              </div>
              {(ep.headers ?? []).map((h, hi) => (
                <div key={hi} className="flex gap-2">
                  <input
                    type="text"
                    value={h.key}
                    onChange={(e) => updateEndpointHeader(index, hi, 'key', e.target.value)}
                    className="input w-40 shrink-0 text-sm"
                    placeholder="Header 名称"
                  />
                  <input
                    type="text"
                    value={h.value}
                    onChange={(e) => updateEndpointHeader(index, hi, 'value', e.target.value)}
                    className="input min-w-0 flex-1 text-sm"
                    placeholder="Header 值"
                  />
                  <Button type="button" variant="ghost" size="icon" onClick={() => removeEndpointHeader(index, hi)}>
                    <Trash2 className="h-4 w-4" />
                  </Button>
                </div>
              ))}
            </div>
            {endpointResults[index] && (
              <div className="rounded-md border bg-background p-3 space-y-1 text-xs">
                {endpointResults[index]!.success ? (
                  <div className="flex flex-wrap items-center gap-3 text-green-600">
                    <span>✓ 成功</span>
                    <span className="text-muted-foreground">{endpointResults[index]!.latency_ms}ms</span>
                    {endpointResults[index]!.time_to_first_token_ms != null && (
                      <span className="text-muted-foreground">TTFT {endpointResults[index]!.time_to_first_token_ms}ms</span>
                    )}
                    {endpointResults[index]!.prompt_tokens != null && endpointResults[index]!.completion_tokens != null && (
                      <span className="text-muted-foreground">{endpointResults[index]!.prompt_tokens}→{endpointResults[index]!.completion_tokens} tok</span>
                    )}
                  </div>
                ) : (
                  <div className="text-red-500">✗ {endpointResults[index]!.error || '测试失败'}</div>
                )}
                {endpointResults[index]!.thinking_detected ? (
                  <div className="text-muted-foreground">
                    <span>检测到思维链（&lt;think&gt; 标签）</span>
                    {endpointResults[index]!.thinking_sample && (
                      <pre className="mt-1 bg-muted p-1 rounded overflow-x-auto whitespace-pre-wrap break-all">{endpointResults[index]!.thinking_sample}</pre>
                    )}
                  </div>
                ) : endpointResults[index]!.success ? (
                  <div className="text-muted-foreground/70">未检测到 &lt;think&gt; 标签</div>
                ) : null}
              </div>
            )}
          </div>
        ))}
      </section>

      {/* API Keys */}
      <section className="space-y-3">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-medium text-muted-foreground">上游 API Keys *</h3>
          <Button type="button" variant="outline" size="sm" onClick={addApiKey}>
            <Plus className="h-4 w-4 mr-1" /> 添加
          </Button>
        </div>
        {apiKeys.map((apiKey, index) => (
          <div key={index} className="space-y-1">
            <div className="flex gap-2 items-center">
              <input
                type="text"
                value={apiKey.key}
                onChange={(e) => updateApiKey(index, 'key', e.target.value)}
                className="input font-mono flex-1"
                placeholder="sk-..."
              />
              <label className="flex items-center gap-1 text-xs text-muted-foreground whitespace-nowrap cursor-pointer">
                <input
                  type="checkbox"
                  checked={apiKey.enabled !== false}
                  onChange={(e) => updateApiKey(index, 'enabled', e.target.checked)}
                  className="rounded"
                />
                启用
              </label>
              {apiKeys.length > 1 && (
                <Button type="button" variant="ghost" size="icon" onClick={() => removeApiKey(index)}>
                  <Trash2 className="h-4 w-4" />
                </Button>
              )}
            </div>
            <input
              type="text"
              value={apiKey.note}
              onChange={(e) => updateApiKey(index, 'note', e.target.value)}
              className="input text-xs w-full"
              placeholder="备注（可选）"
            />
          </div>
        ))}
      </section>

      {/* 模型配置 */}
      <section className="space-y-3">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-medium text-muted-foreground">模型配置</h3>
          <Button type="button" variant="outline" size="sm" onClick={handleFetchModels} disabled={fetchingModels}>
            <RefreshCw className={`h-4 w-4 mr-1 ${fetchingModels ? 'animate-spin' : ''}`} />
            {fetchingModels ? '获取中...' : '获取模型'}
          </Button>
        </div>

        {fetchError && (
          <div className="rounded-lg bg-yellow-50 border border-yellow-200 p-3 text-sm text-yellow-800 dark:bg-yellow-900/20 dark:border-yellow-800 dark:text-yellow-400">
            获取模型失败：{fetchError}，请手动添加
          </div>
        )}

        <div>
          <label className="block text-sm font-medium mb-1">
            可用模型 ({models.length})
          </label>
          <div className="flex gap-2 mb-2">
            <input
              type="text"
              value={manualModelInput}
              onChange={(e) => setManualModelInput(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && (e.preventDefault(), addManualModel())}
              className="input flex-1"
              placeholder="输入模型名称，回车添加"
            />
            <Button type="button" variant="outline" size="sm" onClick={addManualModel}>
              <Plus className="h-4 w-4" />
            </Button>
          </div>
          {models.length > 0 && (
            <div className="max-h-32 overflow-y-auto rounded-lg border bg-muted/30 p-2">
              <div className="flex flex-wrap gap-1">
                {models.map((model) => (
                  <span key={model} className="inline-flex items-center gap-1 px-2 py-1 rounded-md bg-background text-xs border">
                    {model}
                    <button type="button" onClick={() => removeModel(model)} className="hover:text-destructive">
                      <X className="h-3 w-3" />
                    </button>
                  </span>
                ))}
              </div>
            </div>
          )}
        </div>
      </section>

      {/* 高级配置 */}
      <section className="space-y-3">
        <h3 className="text-sm font-medium text-muted-foreground">高级配置</h3>
        <div className="grid grid-cols-2 gap-3">
          <div>
            <label className="block text-sm font-medium mb-1">RPM 限制</label>
            <input type="number" value={rateLimitRpm} onChange={(e) => setRateLimitRpm(e.target.value)} className="input" placeholder="不限" />
          </div>
          <div>
            <label className="block text-sm font-medium mb-1">TPM 限制</label>
            <input type="number" value={rateLimitTpm} onChange={(e) => setRateLimitTpm(e.target.value)} className="input" placeholder="不限" />
          </div>
          <div>
            <label className="block text-sm font-medium mb-1">失败阈值</label>
            <input type="number" value={failureThreshold} onChange={(e) => setFailureThreshold(e.target.value)} className="input" />
          </div>
          <div>
            <label className="block text-sm font-medium mb-1">黑名单分钟</label>
            <input type="number" value={blacklistMinutes} onChange={(e) => setBlacklistMinutes(e.target.value)} className="input" />
          </div>
          <div>
            <label className="block text-sm font-medium mb-1">并发数</label>
            <input type="number" value={concurrency} onChange={(e) => setConcurrency(e.target.value)} className="input" />
          </div>
          <div>
            <label className="block text-sm font-medium mb-1">超时（秒）</label>
            <input type="number" value={timeoutSecs} onChange={(e) => setTimeoutSecs(e.target.value)} className="input" />
          </div>
          <div>
            <label className="block text-sm font-medium mb-1">最大并发<span className="text-muted-foreground font-normal">（可选，0=不限）</span></label>
            <input type="number" value={maxConcurrency} onChange={(e) => setMaxConcurrency(e.target.value)} className="input" placeholder="0=不限" />
          </div>
        </div>
      </section>

      {/* 操作按钮 */}
      <div className="flex justify-end gap-2 pt-2 border-t">
        <Button type="button" variant="outline" onClick={onCancel}>取消</Button>
        <Button type="submit" disabled={submitting} className="btn-primary">
          {submitting ? '保存中...' : channel ? '更新' : '创建'}
        </Button>
      </div>
    </form>
  )
}
