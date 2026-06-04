import { useMemo, useState } from 'react'
import { useModels, useUpdateModel } from '@/api/query-hooks'
import type { ModelInfo } from '@/api/model-info'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter } from '@/components/ui/dialog'
import { Search } from 'lucide-react'
import { toast } from 'sonner'

const CAPABILITY_LABELS: Record<string, string> = {
  supports_function_calling: '函数调用',
  supports_reasoning: '推理',
  supports_vision: '视觉',
  supports_pdf_input: 'PDF',
  supports_prompt_caching: '缓存',
  supports_system_messages: '系统消息',
  supports_tool_choice: '工具选择',
}

const CAPABILITY_KEYS = Object.keys(CAPABILITY_LABELS) as (keyof typeof CAPABILITY_LABELS)[]

const CAPABILITY_COLORS: Record<string, string> = {
  supports_vision: 'bg-blue-100 dark:bg-blue-900/30 text-blue-700 dark:text-blue-400',
  supports_reasoning: 'bg-purple-100 dark:bg-purple-900/30 text-purple-700 dark:text-purple-400',
  supports_function_calling: 'bg-green-100 dark:bg-green-900/30 text-green-700 dark:text-green-400',
  supports_prompt_caching: 'bg-amber-100 dark:bg-amber-900/30 text-amber-700 dark:text-amber-400',
  supports_pdf_input: 'bg-rose-100 dark:bg-rose-900/30 text-rose-700 dark:text-rose-400',
}

export function Models() {
  const { data: models = [], isLoading } = useModels()
  const updateModel = useUpdateModel()
  const [search, setSearch] = useState('')
  const [provider, setProvider] = useState('')
  const [editing, setEditing] = useState<ModelInfo | null>(null)

  const providers = useMemo(() => [...new Set(models.map((m) => m.provider))].sort(), [models])
  const filtered = useMemo(() => models.filter((m) => {
    if (provider && m.provider !== provider) return false
    if (search && !m.model.toLowerCase().includes(search.toLowerCase())) return false
    return true
  }), [models, provider, search])

  const handleSave = async (info: ModelInfo) => {
    try {
      await updateModel.mutateAsync(info)
      toast.success('保存成功')
      setEditing(null)
    } catch (err) {
      toast.error(err instanceof Error ? err.message : '保存失败')
    }
  }

  const fmt = (v: number | null | undefined, prefix = '$') =>
    v != null ? `${prefix}${v.toFixed(2)}` : '-'

  const fmtTokens = (v: number | null | undefined) =>
    v != null ? (v >= 1000000 ? `${(v / 1000000).toFixed(1)}M` : v >= 1000 ? `${(v / 1000).toFixed(0)}K` : String(v)) : '-'

  return (
    <div className="max-w-6xl space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <p className="text-sm text-muted-foreground">共 {models.length} 个模型（数据来自 models.dev，可手动覆盖）</p>
        </div>
      </div>

      {/* 筛选栏 */}
      <div className="flex items-center gap-3">
        <select
          value={provider}
          onChange={(e) => setProvider(e.target.value)}
          className="input w-auto min-w-[120px]"
        >
          <option value="">全部 Provider</option>
          {providers.map((p) => (
            <option key={p} value={p}>{p}</option>
          ))}
        </select>
        <div className="relative flex-1 max-w-sm">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
          <input
            type="text"
            placeholder="搜索模型..."
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="input pl-9"
          />
        </div>
      </div>

      {/* 表格 */}
      <div className="rounded-2xl border bg-card overflow-hidden">
        <div className="max-h-[600px] overflow-y-auto">
          <table className="w-full text-sm">
            <thead className="sticky top-0 bg-card z-10">
              <tr className="border-b bg-muted/50">
                <th className="text-left px-4 py-3 font-medium">模型</th>
                <th className="text-left px-4 py-3 font-medium">Provider</th>
                <th className="text-right px-4 py-3 font-medium">输入</th>
                <th className="text-right px-4 py-3 font-medium">输出</th>
                <th className="text-right px-4 py-3 font-medium">缓存读</th>
                <th className="text-right px-4 py-3 font-medium">缓存写</th>
                <th className="text-right px-4 py-3 font-medium">上下文</th>
                <th className="text-right px-4 py-3 font-medium">输出上限</th>
                <th className="text-center px-4 py-3 font-medium">能力</th>
              </tr>
            </thead>
            <tbody>
              {isLoading ? (
                <tr>
                  <td colSpan={9} className="text-center py-12 text-muted-foreground">加载中...</td>
                </tr>
              ) : filtered.length === 0 ? (
                <tr>
                  <td colSpan={9} className="text-center py-12 text-muted-foreground">
                    {models.length === 0 ? '暂无模型数据，等待远程同步...' : '没有匹配的模型'}
                  </td>
                </tr>
              ) : (
                filtered.map((m) => (
                  <tr
                    key={m.model}
                    className={`border-b last:border-0 hover:bg-muted/30 transition-colors cursor-pointer ${editing?.model === m.model ? 'bg-muted/30' : ''}`}
                    onClick={() => setEditing(m)}
                  >
                    <td className="px-4 py-3 font-mono text-xs">{m.model}</td>
                    <td className="px-4 py-3 text-xs text-muted-foreground">{m.provider}</td>
                    <td className="px-4 py-3 text-right tabular-nums text-xs">{fmt(m.input_price)}</td>
                    <td className="px-4 py-3 text-right tabular-nums text-xs">{fmt(m.output_price)}</td>
                    <td className="px-4 py-3 text-right tabular-nums text-xs text-muted-foreground">{fmt(m.cache_read_price)}</td>
                    <td className="px-4 py-3 text-right tabular-nums text-xs text-muted-foreground">{fmt(m.cache_creation_price)}</td>
                    <td className="px-4 py-3 text-right tabular-nums text-xs">{fmtTokens(m.max_input_tokens)}</td>
                    <td className="px-4 py-3 text-right tabular-nums text-xs">{fmtTokens(m.max_output_tokens)}</td>
                    <td className="px-4 py-3 text-center">
                      <div className="flex flex-wrap gap-0.5 justify-center">
                        {CAPABILITY_KEYS.filter((k) => k in CAPABILITY_COLORS && m[k as keyof ModelInfo]).map((k) => (
                          <span key={k} className={`text-[10px] px-1 py-0.5 rounded ${CAPABILITY_COLORS[k]}`}>{CAPABILITY_LABELS[k]}</span>
                        ))}
                      </div>
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      </div>

      <p className="text-xs text-muted-foreground px-1">
        价格单位：USD / 1M tokens。点击行可编辑。数据由 models.dev 提供，按配置定时刷新。
      </p>

      {editing && (
        <ModelEditDialog
          model={editing}
          onSave={handleSave}
          onClose={() => setEditing(null)}
        />
      )}
    </div>
  )
}

function ModelEditDialog({ model, onSave, onClose }: {
  model: ModelInfo
  onSave: (m: ModelInfo) => Promise<void>
  onClose: () => void
}) {
  const [draft, setDraft] = useState<ModelInfo>(model)
  const [saving, setSaving] = useState(false)

  const handleSave = async () => {
    setSaving(true)
    try { await onSave(draft) } finally { setSaving(false) }
  }

  return (
    <Dialog open onOpenChange={(open) => { if (!open) onClose() }}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle className="font-mono text-sm">{model.model}</DialogTitle>
        </DialogHeader>

        <div className="grid grid-cols-2 gap-4 py-2">
          <div className="space-y-3">
            <h4 className="text-xs font-medium text-muted-foreground">定价（$/1M tokens）</h4>
            <div className="grid grid-cols-2 gap-2">
              <label className="text-xs text-muted-foreground">输入
                <input type="number" step="0.01" className="input w-full text-xs mt-0.5"
                  value={draft.input_price ?? ''} onChange={(e) => setDraft({ ...draft, input_price: e.target.value ? parseFloat(e.target.value) : null })} />
              </label>
              <label className="text-xs text-muted-foreground">输出
                <input type="number" step="0.01" className="input w-full text-xs mt-0.5"
                  value={draft.output_price ?? ''} onChange={(e) => setDraft({ ...draft, output_price: e.target.value ? parseFloat(e.target.value) : null })} />
              </label>
              <label className="text-xs text-muted-foreground">缓存读取
                <input type="number" step="0.01" className="input w-full text-xs mt-0.5"
                  value={draft.cache_read_price ?? ''} onChange={(e) => setDraft({ ...draft, cache_read_price: e.target.value ? parseFloat(e.target.value) : null })} />
              </label>
              <label className="text-xs text-muted-foreground">缓存写入
                <input type="number" step="0.01" className="input w-full text-xs mt-0.5"
                  value={draft.cache_creation_price ?? ''} onChange={(e) => setDraft({ ...draft, cache_creation_price: e.target.value ? parseFloat(e.target.value) : null })} />
              </label>
            </div>
          </div>

          <div className="space-y-3">
            <h4 className="text-xs font-medium text-muted-foreground">上下文窗口</h4>
            <div className="grid grid-cols-2 gap-2">
              <label className="text-xs text-muted-foreground">最大输入 tokens
                <input type="number" className="input w-full text-xs mt-0.5"
                  value={draft.max_input_tokens ?? ''} onChange={(e) => setDraft({ ...draft, max_input_tokens: e.target.value ? parseInt(e.target.value) : null })} />
              </label>
              <label className="text-xs text-muted-foreground">最大输出 tokens
                <input type="number" className="input w-full text-xs mt-0.5"
                  value={draft.max_output_tokens ?? ''} onChange={(e) => setDraft({ ...draft, max_output_tokens: e.target.value ? parseInt(e.target.value) : null })} />
              </label>
            </div>
          </div>
        </div>

        <div className="space-y-2 py-1">
          <h4 className="text-xs font-medium text-muted-foreground">能力</h4>
          <div className="flex flex-wrap gap-2">
            {CAPABILITY_KEYS.map((key) => (
                <button key={key} type="button"
                  onClick={() => setDraft({ ...draft, [key]: (draft as unknown as Record<string, unknown>)[key] === true ? null : true })}
                  className={`text-xs px-2 py-1 rounded border transition-colors ${
                    (draft as unknown as Record<string, unknown>)[key] === true
                      ? 'bg-primary text-primary-foreground border-primary'
                      : 'bg-card text-muted-foreground border-border'
                  }`}>
                  {CAPABILITY_LABELS[key]}
                </button>
            ))}
          </div>
        </div>

        <DialogFooter>
          <Button size="sm" variant="ghost" onClick={onClose}>取消</Button>
          <Button size="sm" onClick={handleSave} disabled={saving} className="btn-primary">
            {saving ? '保存中...' : '保存'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
