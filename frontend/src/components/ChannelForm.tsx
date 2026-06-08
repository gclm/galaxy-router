import { useState } from 'react'
import type { Channel, CreateChannelRequest, CustomHeader, EndpointConfig, EndpointType, UpstreamApiKey } from '@/api/types'
import { ENDPOINT_LABELS } from '@/api/types'
import { channelsApi } from '@/api/channels'
import { Button } from '@/components/ui/button'
import { Plus, Trash2, RefreshCw, X } from 'lucide-react'

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

/** 将 dot-notation 扁平 key 展开为嵌套对象
 *  例: { "a.b": 1, "c": 2 } => { a: { b: 1 }, c: 2 } */
function expandDotKeys(flat: Record<string, unknown>): Record<string, unknown> {
  const result: Record<string, unknown> = {}
  for (const [k, v] of Object.entries(flat)) {
    const parts = k.split('.')
    let cur: Record<string, unknown> = result
    for (let i = 0; i < parts.length - 1; i++) {
      const p = parts[i]
      if (typeof cur[p] !== 'object' || cur[p] === null) cur[p] = {}
      cur = cur[p] as Record<string, unknown>
    }
    cur[parts[parts.length - 1]] = v
  }
  return result
}

/** 递归合并两个对象（b 覆盖 a） */
function deepMerge<T extends Record<string, unknown>>(a: T, b: Record<string, unknown>): T {
  const result: Record<string, unknown> = { ...a }
  for (const [k, v] of Object.entries(b)) {
    const av = result[k]
    if (
      av && typeof av === 'object' && !Array.isArray(av) &&
      v && typeof v === 'object' && !Array.isArray(v)
    ) {
      result[k] = deepMerge(av as Record<string, unknown>, v as Record<string, unknown>)
    } else {
      result[k] = v
    }
  }
  return result as T
}

export function ChannelForm({ channel, onSubmit, onCancel }: ChannelFormProps) {
  const [name, setName] = useState(channel?.name ?? '')
  const [apiKeys, setApiKeys] = useState<UpstreamApiKey[]>(
    channel?.api_keys?.map(k => typeof k === 'string' ? { key: k, note: '', enabled: true } : { key: k.key, note: k.note ?? '', enabled: k.enabled ?? true }) ?? [{ key: '', note: '', enabled: true }]
  )
  const [endpoints, setEndpoints] = useState<EndpointConfig[]>(
    channel?.endpoints ?? [{ type: 'openai_chat', base_url: '', enabled: true }]
  )
  const [models, setModels] = useState<string[]>(channel?.models ?? [])
  const [rateLimitRpm, setRateLimitRpm] = useState(channel?.rate_limit_rpm?.toString() ?? '')
  const [rateLimitTpm, setRateLimitTpm] = useState(channel?.rate_limit_tpm?.toString() ?? '')
  const [failureThreshold, setFailureThreshold] = useState(channel?.failure_threshold?.toString() ?? '3')
  const [blacklistMinutes, setBlacklistMinutes] = useState(channel?.blacklist_minutes?.toString() ?? '5')
  const [concurrency, setConcurrency] = useState(channel?.concurrency?.toString() ?? '10')
  const [timeoutSecs, setTimeoutSecs] = useState(channel?.timeout_secs?.toString() ?? '300')
  const [maxConcurrency, setMaxConcurrency] = useState(channel?.max_concurrency?.toString() ?? '0')
  const [customHeaders, setCustomHeaders] = useState<CustomHeader[]>(channel?.custom_headers ?? [])
  // extras 编辑：字符串形式以便 textarea 编辑，提交时再 parse
  const [extrasRaw, setExtrasRaw] = useState(() => {
    if (!channel?.extras) return ''
    return JSON.stringify(channel.extras, null, 2)
  })
  const [extrasError, setExtrasError] = useState('')
  // Detect 状态
  const [detecting, setDetecting] = useState(false)
  const [detectResult, setDetectResult] = useState<import('@/api/types').DetectResponse | null>(null)
  const [detectError, setDetectError] = useState('')
  const [enabled, setEnabled] = useState(channel?.enabled ?? true)
  const [submitting, setSubmitting] = useState(false)
  const [fetchingModels, setFetchingModels] = useState(false)
  const [fetchError, setFetchError] = useState('')
  const [manualModelInput, setManualModelInput] = useState('')

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
        endpoints: endpoints.filter((ep) => ep.base_url.trim()).map(ep => ({ type: ep.type, base_url: ep.base_url, enabled: ep.enabled })),
        models,
        enabled,
        failure_threshold: parseInt(failureThreshold) || 3,
        blacklist_minutes: parseInt(blacklistMinutes) || 5,
        concurrency: parseInt(concurrency) || 10,
        timeout_secs: parseInt(timeoutSecs) || 300,
        max_concurrency: parseInt(maxConcurrency) || 0,
        custom_headers: customHeaders.filter((h) => h.key.trim()),
      }

      if (rateLimitRpm) data.rate_limit_rpm = parseInt(rateLimitRpm)
      if (rateLimitTpm) data.rate_limit_tpm = parseInt(rateLimitTpm)

      // 解析 extras JSON
      const trimmedExtras = extrasRaw.trim()
      if (trimmedExtras) {
        try {
          const parsed = JSON.parse(trimmedExtras)
          if (typeof parsed !== 'object' || Array.isArray(parsed) || parsed === null) {
            setExtrasError('extras 必须是 JSON 对象')
            return
          }
          data.extras = parsed as Record<string, unknown>
          setExtrasError('')
        } catch (e) {
          setExtrasError(`extras JSON 解析失败: ${e instanceof Error ? e.message : '未知错误'}`)
          return
        }
      }

      await onSubmit(data)
    } finally {
      setSubmitting(false)
    }
  }

  const handleDetect = async () => {
    if (!channel?.id) {
      setDetectError('请先保存渠道后再检测')
      return
    }
    setDetecting(true)
    setDetectError('')
    setDetectResult(null)
    try {
      const result = await channelsApi.detectQuirks(channel.id, {})
      setDetectResult(result)
    } catch (e: unknown) {
      setDetectError(e instanceof Error ? e.message : '检测失败')
    } finally {
      setDetecting(false)
    }
  }

  const applyDetectedRecommendations = () => {
    if (!detectResult) return
    // 合并到现有 extras
    // 检测 API 返回的 recommendations 是 dot-notation 扁平 key（如 "thinking.extract_tags"），
    // 需要展开为嵌套对象（{thinking: {extract_tags: true}}），与后端解析逻辑对齐
    const current: Record<string, unknown> = extrasRaw.trim()
      ? (() => { try { return JSON.parse(extrasRaw) } catch { return {} } })()
      : {}
    const merged = deepMerge(current, expandDotKeys(detectResult.recommendations))
    setExtrasRaw(JSON.stringify(merged, null, 2))
    setDetectResult(null)
  }

  const addApiKey = () => setApiKeys([...apiKeys, { key: '', note: '', enabled: true }])
  const removeApiKey = (index: number) => setApiKeys(apiKeys.filter((_, i) => i !== index))
  const updateApiKey = (index: number, field: keyof UpstreamApiKey, value: string | boolean) => {
    const newKeys = [...apiKeys]
    newKeys[index] = { ...newKeys[index], [field]: value }
    setApiKeys(newKeys)
  }

  const addEndpoint = () => setEndpoints([...endpoints, { type: 'openai_chat', base_url: '', enabled: true }])
  const removeEndpoint = (index: number) => setEndpoints(endpoints.filter((_, i) => i !== index))
  const updateEndpoint = (index: number, field: keyof EndpointConfig, value: string | boolean) => {
    const newEndpoints = [...endpoints]
    newEndpoints[index] = { ...newEndpoints[index], [field]: value }
    setEndpoints(newEndpoints)
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
          <div key={index} className="flex gap-2 items-center">
            <select
              value={ep.type}
              onChange={(e) => updateEndpoint(index, 'type', e.target.value)}
              className="input w-40"
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
            {endpoints.length > 1 && (
              <Button type="button" variant="ghost" size="icon" onClick={() => removeEndpoint(index)}>
                <Trash2 className="h-4 w-4" />
              </Button>
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
            <label className="block text-sm font-medium mb-1">最大并发</label>
            <input type="number" value={maxConcurrency} onChange={(e) => setMaxConcurrency(e.target.value)} className="input" placeholder="0=不限" />
          </div>
        </div>
      </section>

      {/* 自定义请求头 */}
      <section className="space-y-3">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-medium text-muted-foreground">自定义请求头</h3>
          <Button type="button" variant="outline" size="sm" onClick={() => setCustomHeaders([...customHeaders, { key: '', value: '' }])}>
            <Plus className="h-4 w-4 mr-1" /> 添加
          </Button>
        </div>
        {customHeaders.map((h, i) => (
          <div key={i} className="flex gap-2">
            <input
              type="text"
              value={h.key}
              onChange={(e) => {
                const updated = [...customHeaders]
                updated[i] = { ...updated[i], key: e.target.value }
                setCustomHeaders(updated)
              }}
              className="input w-40"
              placeholder="Header 名称"
            />
            <input
              type="text"
              value={h.value}
              onChange={(e) => {
                const updated = [...customHeaders]
                updated[i] = { ...updated[i], value: e.target.value }
                setCustomHeaders(updated)
              }}
              className="input flex-1"
              placeholder="Header 值"
            />
            <Button type="button" variant="ghost" size="icon" onClick={() => setCustomHeaders(customHeaders.filter((_, j) => j !== i))}>
              <Trash2 className="h-4 w-4" />
            </Button>
          </div>
        ))}
      </section>

      {/* 扩展设置（extras） */}
      <section className="space-y-3">
        <div className="flex items-center justify-between">
          <div>
            <h3 className="text-sm font-medium text-muted-foreground">扩展设置（extras）</h3>
            <p className="text-xs text-muted-foreground mt-1">
              JSON 自由格式。当前支持 thinking.extract_tags（抽取 &lt;think/&gt; 标签）和
              thinking.fix_signature（修复 GLM-style signature 位置）。
            </p>
          </div>
          {channel && (
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={handleDetect}
              disabled={detecting}
            >
              <RefreshCw className={`h-4 w-4 mr-1 ${detecting ? 'animate-spin' : ''}`} />
              {detecting ? '检测中...' : '检测上游行为'}
            </Button>
          )}
        </div>
        <textarea
          value={extrasRaw}
          onChange={(e) => {
            setExtrasRaw(e.target.value)
            setExtrasError('')
          }}
          className={`input font-mono text-sm ${extrasError ? 'border-red-500' : ''}`}
          rows={5}
          placeholder='{"thinking": {"extract_tags": true, "fix_signature": false}}'
        />
        {extrasError && <p className="text-xs text-red-500">{extrasError}</p>}

        {detectError && (
          <p className="text-xs text-red-500">检测失败：{detectError}</p>
        )}

        {detectResult && (
          <div className="rounded-md border bg-muted/30 p-3 space-y-2 text-sm">
            <div className="flex items-center justify-between">
              <span className="font-medium">渠道级合并推荐</span>
              <Button type="button" variant="outline" size="sm" onClick={applyDetectedRecommendations}>
                应用到 extras
              </Button>
            </div>
            {Object.keys(detectResult.recommendations).length === 0 ? (
              <p className="text-xs text-muted-foreground">
                未检测到需要开启的修复项（上游实现规范）。
              </p>
            ) : (
              <ul className="text-xs space-y-0.5">
                {Object.entries(detectResult.recommendations).map(([k, v]) => (
                  <li key={k}>
                    <code className="bg-muted px-1 rounded">{k}</code>:{' '}
                    <span className={v ? 'text-green-600 font-medium' : 'text-muted-foreground'}>
                      {v ? 'true' : 'false'}
                    </span>
                  </li>
                ))}
              </ul>
            )}
            <details className="text-xs">
              <summary className="cursor-pointer text-muted-foreground hover:text-foreground">
                各端点详情
              </summary>
              <div className="mt-2 space-y-2">
                {detectResult.endpoint_results.map((r) => (
                  <div key={r.endpoint} className="border-l-2 pl-2">
                    <div className="font-mono text-xs text-muted-foreground">{r.endpoint}</div>
                    {r.evidence && <div className="text-xs">证据：{r.evidence}</div>}
                    {r.sample && (
                      <pre className="text-xs bg-muted p-1 rounded mt-1 overflow-x-auto">
                        {r.sample}
                      </pre>
                    )}
                    {Object.keys(r.recommendations).length === 0 && (
                      <div className="text-xs text-muted-foreground">无推荐</div>
                    )}
                  </div>
                ))}
              </div>
            </details>
          </div>
        )}
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
