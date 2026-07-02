import { useState, useMemo } from 'react'
import { useLocation } from 'react-router-dom'
import { useApiKeys, useGroups } from '@/api/query-hooks'
import { copyText } from '@/lib/utils'
import { generateClaudeConfig, generateCodexConfig } from '@/lib/clientConfig'
import { toast } from 'sonner'
import { Copy, Download, ChevronRight } from 'lucide-react'

type ClientType = 'cc' | 'codex'

const inputCls = 'w-full rounded-md border border-input bg-background px-3 py-2 text-sm'
const labelCls = 'block text-sm font-medium mb-1.5'

export function ClientConfig() {
  const { data: keysData } = useApiKeys()
  const { data: groupsData } = useGroups()

  const keys = useMemo(
    () => (keysData?.items ?? []).filter((k) => k.enabled),
    [keysData],
  )
  const groupNames = useMemo(
    () => (groupsData?.items ?? []).filter((g) => g.enabled).map((g) => g.name),
    [groupsData],
  )

  // 创建 key 成功后跳转过来时，通过路由 state 预填 key（见 ApiKeyForm 接入）
  const location = useLocation()
  const presetKey = (location.state as { apiKey?: string } | null)?.apiKey ?? ''

  const [clientType, setClientType] = useState<ClientType>('cc')
  const [baseUrl, setBaseUrl] = useState(window.location.origin)
  // 受控输入：用户手改时存这里；未改时派生默认（presetKey / 第一个 key / 第一个 group）
  const [apiKeyInput, setApiKeyInput] = useState('')
  const [sonnetInput, setSonnetInput] = useState('')
  const [opusInput, setOpusInput] = useState('')
  const [haikuInput, setHaikuInput] = useState('')
  const [codexModelInput, setCodexModelInput] = useState('')
  const [hideAttribution, setHideAttribution] = useState(false)
  const [effortMax, setEffortMax] = useState(false)
  const [disableAutoUpdate, setDisableAutoUpdate] = useState(false)
  const [showRaw, setShowRaw] = useState(false)

  const apiKey = apiKeyInput || presetKey || keys[0]?.api_key || ''
  const defaultGroup = groupNames[0] ?? ''
  const sonnet = sonnetInput || defaultGroup
  const opus = opusInput || defaultGroup
  const haiku = haikuInput || defaultGroup
  const codexModel = codexModelInput || defaultGroup

  const claudeConfig = useMemo(
    () => generateClaudeConfig({ baseUrl, apiKey, sonnet, opus, haiku, hideAttribution, effortMax, disableAutoUpdate }),
    [baseUrl, apiKey, sonnet, opus, haiku, hideAttribution, effortMax, disableAutoUpdate],
  )
  const codexConfig = useMemo(
    () => generateCodexConfig({ baseUrl, apiKey, model: codexModel }),
    [baseUrl, apiKey, codexModel],
  )

  const previewText = clientType === 'cc'
    ? JSON.stringify(claudeConfig, null, 2)
    : codexConfig.files.map((f) => `# ${f.path}\n${f.content}`).join('\n\n')

  async function handleCopy() {
    const ok = await copyText(previewText)
    if (ok) toast.success('已复制到剪贴板')
    else toast.error('复制失败，请手动选中复制')
  }

  function handleDownload() {
    const filename = clientType === 'cc' ? 'settings.json' : 'codex-config.txt'
    const blob = new Blob([previewText], { type: 'text/plain;charset=utf-8' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = filename
    a.click()
    URL.revokeObjectURL(url)
    toast.success(`已下载 ${filename}`)
  }

  const noKey = keys.length === 0
  const noGroup = groupNames.length === 0

  return (
    <div className="space-y-4">
      <div>
        <h1 className="text-xl font-semibold">客户端配置</h1>
        <p className="text-xs text-muted-foreground mt-1">
          选好 API Key 和客户端类型，一键生成可直接粘贴的配置。网关地址默认取当前浏览器地址，可手动修改。
        </p>
      </div>

      {/* 客户端类型 Tab */}
      <div className="flex gap-2">
        {(['cc', 'codex'] as const).map((t) => (
          <button
            key={t}
            onClick={() => setClientType(t)}
            className={`px-4 py-2 rounded-md text-sm font-medium transition-colors ${
              clientType === t ? 'bg-primary text-primary-foreground' : 'bg-muted text-muted-foreground hover:bg-muted/80'
            }`}
          >
            {t === 'cc' ? 'Claude Code' : 'Codex'}
          </button>
        ))}
      </div>

      {/* 表单 */}
      <div className="rounded-lg border border-border bg-card p-6 space-y-5">
        <div>
          <label className={labelCls}>API Key</label>
          {noKey ? (
            <p className="text-sm text-muted-foreground">暂无启用的 API Key，请先在「API Keys」页面创建。</p>
          ) : (
            <select className={inputCls} value={apiKey} onChange={(e) => setApiKeyInput(e.target.value)}>
              {keys.map((k) => (
                <option key={k.id} value={k.api_key}>{k.name}（{k.api_key}）</option>
              ))}
            </select>
          )}
        </div>

        <div>
          <label className={labelCls}>网关地址</label>
          <input
            className={inputCls}
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
            placeholder="https://router.example.com"
          />
        </div>

        {clientType === 'cc' ? (
          <div className="space-y-3">
            <div className="text-sm font-medium pt-1">
              模型档位映射 <span className="text-xs text-muted-foreground font-normal">（每个档位选一个虚拟模型 / group）</span>
            </div>
            {noGroup ? (
              <p className="text-sm text-muted-foreground">暂无启用的分组（group），请先在「分组」页面创建。</p>
            ) : (
              <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
                <div>
                  <label className="block text-xs text-muted-foreground mb-1">Sonnet 档</label>
                  <select className={inputCls} value={sonnet} onChange={(e) => setSonnetInput(e.target.value)}>
                    {groupNames.map((n) => <option key={n} value={n}>{n}</option>)}
                  </select>
                </div>
                <div>
                  <label className="block text-xs text-muted-foreground mb-1">Opus 档</label>
                  <select className={inputCls} value={opus} onChange={(e) => setOpusInput(e.target.value)}>
                    {groupNames.map((n) => <option key={n} value={n}>{n}</option>)}
                  </select>
                </div>
                <div>
                  <label className="block text-xs text-muted-foreground mb-1">Haiku 档</label>
                  <select className={inputCls} value={haiku} onChange={(e) => setHaikuInput(e.target.value)}>
                    {groupNames.map((n) => <option key={n} value={n}>{n}</option>)}
                  </select>
                </div>
              </div>
            )}
          </div>
        ) : (
          <div className="space-y-3">
            <div className="text-sm font-medium pt-1">
              默认模型 <span className="text-xs text-muted-foreground font-normal">（虚拟模型 / group）</span>
            </div>
            {noGroup ? (
              <p className="text-sm text-muted-foreground">暂无启用的分组（group），请先在「分组」页面创建。</p>
            ) : (
              <select className={inputCls} value={codexModel} onChange={(e) => setCodexModelInput(e.target.value)}>
                {groupNames.map((n) => <option key={n} value={n}>{n}</option>)}
              </select>
            )}
          </div>
        )}

        {clientType === 'cc' && (
          <div className="pt-1">
            <div className="text-sm font-medium mb-2">选项</div>
            <div className="flex flex-wrap gap-x-6 gap-y-2 text-sm">
              <label className="flex items-center gap-2">
                <input type="checkbox" checked={hideAttribution} onChange={(e) => setHideAttribution(e.target.checked)} />
                hideAttribution（隐藏署名）
              </label>
              <label className="flex items-center gap-2">
                <input type="checkbox" checked={effortMax} onChange={(e) => setEffortMax(e.target.checked)} />
                effortMax（最高思考强度）
              </label>
              <label className="flex items-center gap-2">
                <input type="checkbox" checked={disableAutoUpdate} onChange={(e) => setDisableAutoUpdate(e.target.checked)} />
                disableAutoUpdate（禁用自动更新）
              </label>
            </div>
          </div>
        )}
      </div>

      {/* 生成结果 */}
      <div className="rounded-lg border border-border bg-card p-6">
        <div className="flex items-center justify-between mb-3">
          <div className="text-sm font-medium">生成的配置</div>
          <div className="flex gap-2">
            <button
              onClick={handleCopy}
              className="inline-flex items-center gap-1.5 rounded-md bg-primary text-primary-foreground px-3 py-1.5 text-xs hover:bg-primary/90"
            >
              <Copy size={14} />复制
            </button>
            <button
              onClick={handleDownload}
              className="inline-flex items-center gap-1.5 rounded-md border border-border px-3 py-1.5 text-xs hover:bg-muted"
            >
              <Download size={14} />下载
            </button>
          </div>
        </div>
        <pre className="text-xs font-mono bg-muted/50 border border-border rounded-md p-4 overflow-auto max-h-80 leading-relaxed whitespace-pre-wrap break-all">
          {previewText}
        </pre>
        <button
          onClick={() => setShowRaw((v) => !v)}
          className="mt-3 inline-flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
        >
          <ChevronRight size={12} className={showRaw ? 'rotate-90 transition-transform' : 'transition-transform'} />
          原始 JSON（高级，可手动编辑后带走）
        </button>
        {showRaw && (
          <pre className="text-xs font-mono bg-muted/50 border border-border rounded-md p-4 mt-2 overflow-auto max-h-64">
            {clientType === 'cc' ? JSON.stringify(claudeConfig, null, 2) : JSON.stringify(codexConfig, null, 2)}
          </pre>
        )}
      </div>

      {/* 安装引导 */}
      <div className="rounded-lg border border-border bg-card p-6">
        <details>
          <summary className="text-sm font-medium cursor-pointer select-none">如何安装</summary>
          <div className="mt-3 text-sm text-muted-foreground space-y-2">
            {clientType === 'cc' ? (
              <>
                <p>1. 打开或新建文件 <code className="font-mono bg-muted px-1 rounded">~/.claude/settings.json</code></p>
                <p>2. 若文件已存在：把上面的 <code className="font-mono bg-muted px-1 rounded">env</code> 字段<strong className="text-foreground">合并</strong>进现有 env，不要整体覆盖你已有的配置</p>
                <p>3. 若不存在：直接粘贴整段，保存即可</p>
                <p>4. 重启 Claude Code 生效</p>
              </>
            ) : (
              <>
                <p>Codex 需要两个文件：</p>
                <p>1. <code className="font-mono bg-muted px-1 rounded">~/.codex/config.toml</code> — 粘贴上半段（指向网关）</p>
                <p>2. <code className="font-mono bg-muted px-1 rounded">~/.codex/.env</code> — 粘贴下半段（存 key）；也可改为在 shell 里 <code className="font-mono bg-muted px-1 rounded">export GALAXY_API_KEY=...</code></p>
                <p>3. 重启 Codex 生效</p>
              </>
            )}
          </div>
        </details>
      </div>
    </div>
  )
}
