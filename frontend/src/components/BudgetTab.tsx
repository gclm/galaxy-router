import { useEffect, useState } from 'react'
import { statsApi, apiKeysApi } from '@/api'
import type { BudgetLimit, ApiKey } from '@/api/types'
import { Button } from '@/components/ui/button'
import { ToggleSwitch } from '@/components/ToggleSwitch'
import { DollarSign, Plus, Trash2 } from 'lucide-react'

export function BudgetTab() {
  const [budgets, setBudgets] = useState<BudgetLimit[]>([])
  const [apiKeys, setApiKeys] = useState<ApiKey[]>([])
  const [loading, setLoading] = useState(true)

  const loadData = async () => {
    const [b, k] = await Promise.all([
      statsApi.listBudgets().catch<BudgetLimit[]>(() => []),
      apiKeysApi.list().catch<ApiKey[]>(() => []),
    ])
    setBudgets(b)
    setApiKeys(k)
    setLoading(false)
  }

  useEffect(() => { loadData() }, [])

  const keyNameMap = Object.fromEntries(apiKeys.map(k => [k.id, k.name]))

  const keysWithoutBudget = apiKeys.filter(k => !budgets.some(b => b.api_key_id === k.id))

  const handleToggle = async (budget: BudgetLimit) => {
    try {
      await statsApi.setBudget({
        api_key_id: budget.api_key_id,
        monthly_limit_usd: budget.monthly_limit_usd,
        daily_limit_usd: budget.daily_limit_usd,
        enabled: !budget.enabled,
      })
      await loadData()
    } catch (err) {
      alert(err instanceof Error ? err.message : '操作失败')
    }
  }

  const handleDelete = async (id: string) => {
    if (!confirm('确认删除此预算限制？')) return
    try {
      await statsApi.deleteBudget(id)
      await loadData()
    } catch (err) {
      alert(err instanceof Error ? err.message : '删除失败')
    }
  }

  if (loading) {
    return <div className="py-8 text-center text-sm text-muted-foreground">加载中...</div>
  }

  return (
    <div className="space-y-4">
      <section className="rounded-2xl border bg-card p-5 space-y-4">
        <div className="flex items-center justify-between">
          <h2 className="text-sm font-medium text-muted-foreground flex items-center gap-2">
            <DollarSign className="h-4 w-4" />
            预算限制
          </h2>
          {keysWithoutBudget.length > 0 && (
            <AddBudgetDialog
              availableKeys={keysWithoutBudget}
              onCreated={loadData}
            />
          )}
        </div>

        <p className="text-xs text-muted-foreground">
          为 API Key 设置月度和日度消费上限，超出额度后请求将被拒绝。
        </p>

        {budgets.length === 0 ? (
          <div className="py-6 text-center text-sm text-muted-foreground">
            暂无预算限制{apiKeys.length === 0 ? '，请先创建 API Key' : '，点击右上角添加'}
          </div>
        ) : (
          <div className="divide-y rounded-xl border">
            {budgets.map((budget) => (
              <BudgetRow
                key={budget.id}
                budget={budget}
                keyName={keyNameMap[budget.api_key_id] || budget.api_key_id.slice(0, 8)}
                onToggle={() => handleToggle(budget)}
                onDelete={() => handleDelete(budget.id)}
                onUpdate={loadData}
              />
            ))}
          </div>
        )}
      </section>
    </div>
  )
}

/* ── 单行预算 ── */

function BudgetRow({
  budget,
  keyName,
  onToggle,
  onDelete,
  onUpdate,
}: {
  budget: BudgetLimit
  keyName: string
  onToggle: () => void
  onDelete: () => void
  onUpdate: () => void
}) {
  const [editingField, setEditingField] = useState<'monthly' | 'daily' | null>(null)
  const [draftValue, setDraftValue] = useState('')
  const [pending, setPending] = useState(false)

  const startEdit = (field: 'monthly' | 'daily') => {
    const current = field === 'monthly' ? budget.monthly_limit_usd : budget.daily_limit_usd
    setDraftValue(current > 0 ? String(current) : '')
    setEditingField(field)
  }

  const saveEdit = async () => {
    const num = parseFloat(draftValue)
    if (isNaN(num) || num < 0) return
    setPending(true)
    try {
      await statsApi.setBudget({
        api_key_id: budget.api_key_id,
        monthly_limit_usd: editingField === 'monthly' ? num : budget.monthly_limit_usd,
        daily_limit_usd: editingField === 'daily' ? num : budget.daily_limit_usd,
        enabled: budget.enabled,
      })
      setEditingField(null)
      onUpdate()
    } catch (err) {
      alert(err instanceof Error ? err.message : '保存失败')
    } finally {
      setPending(false)
    }
  }

  return (
    <div className="flex items-center gap-4 px-4 py-3">
      <ToggleSwitch enabled={budget.enabled} onClick={onToggle} size="sm" />

      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium truncate">{keyName}</p>
      </div>

      <div className="flex items-center gap-4 text-sm shrink-0">
        {editingField === 'monthly' ? (
          <InlineEditor
            value={draftValue}
            onChange={setDraftValue}
            onSave={saveEdit}
            onCancel={() => setEditingField(null)}
            pending={pending}
            label="月"
          />
        ) : (
          <button
            type="button"
            onClick={() => startEdit('monthly')}
            className="rounded-lg bg-muted px-2.5 py-1 text-xs font-medium hover:bg-muted/80 transition-colors"
          >
            月 ${budget.monthly_limit_usd > 0 ? budget.monthly_limit_usd.toFixed(2) : '—'}
          </button>
        )}

        {editingField === 'daily' ? (
          <InlineEditor
            value={draftValue}
            onChange={setDraftValue}
            onSave={saveEdit}
            onCancel={() => setEditingField(null)}
            pending={pending}
            label="日"
          />
        ) : (
          <button
            type="button"
            onClick={() => startEdit('daily')}
            className="rounded-lg bg-muted px-2.5 py-1 text-xs font-medium hover:bg-muted/80 transition-colors"
          >
            日 ${budget.daily_limit_usd > 0 ? budget.daily_limit_usd.toFixed(2) : '—'}
          </button>
        )}

        <button
          type="button"
          onClick={onDelete}
          className="text-muted-foreground hover:text-destructive transition-colors p-1"
        >
          <Trash2 className="h-3.5 w-3.5" />
        </button>
      </div>
    </div>
  )
}

/* ── 内联编辑器 ── */

function InlineEditor({
  value,
  onChange,
  onSave,
  onCancel,
  pending,
  label,
}: {
  value: string
  onChange: (v: string) => void
  onSave: () => void
  onCancel: () => void
  pending: boolean
  label: string
}) {
  return (
    <div className="flex items-center gap-1">
      <span className="text-xs text-muted-foreground">{label} $</span>
      <input
        type="number"
        min={0}
        step={0.01}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => { if (e.key === 'Enter') onSave(); if (e.key === 'Escape') onCancel() }}
        autoFocus
        disabled={pending}
        className="input w-20 text-xs py-1"
      />
      <Button size="sm" onClick={onSave} disabled={pending} className="h-6 px-2 text-xs">✓</Button>
    </div>
  )
}

/* ── 新增预算对话框 ── */

function AddBudgetDialog({
  availableKeys,
  onCreated,
}: {
  availableKeys: ApiKey[]
  onCreated: () => void
}) {
  const [open, setOpen] = useState(false)
  const [selectedKeyId, setSelectedKeyId] = useState('')
  const [monthly, setMonthly] = useState('')
  const [daily, setDaily] = useState('')
  const [pending, setPending] = useState(false)

  const handleSubmit = async () => {
    if (!selectedKeyId) return
    const monthlyVal = parseFloat(monthly) || 0
    const dailyVal = parseFloat(daily) || 0
    if (monthlyVal <= 0 && dailyVal <= 0) return
    setPending(true)
    try {
      await statsApi.setBudget({
        api_key_id: selectedKeyId,
        monthly_limit_usd: monthlyVal,
        daily_limit_usd: dailyVal,
        enabled: true,
      })
      setOpen(false)
      setSelectedKeyId('')
      setMonthly('')
      setDaily('')
      onCreated()
    } catch (err) {
      alert(err instanceof Error ? err.message : '创建失败')
    } finally {
      setPending(false)
    }
  }

  if (!open) {
    return (
      <Button size="sm" onClick={() => setOpen(true)}>
        <Plus className="h-3.5 w-3.5 mr-1" />
        添加预算
      </Button>
    )
  }

  return (
    <div className="flex items-center gap-2 flex-wrap">
      <select
        value={selectedKeyId}
        onChange={(e) => setSelectedKeyId(e.target.value)}
        className="input w-auto min-w-[120px] text-xs py-1"
      >
        <option value="">选择 API Key</option>
        {availableKeys.map(k => (
          <option key={k.id} value={k.id}>{k.name}</option>
        ))}
      </select>
      <input
        type="number"
        min={0}
        step={0.01}
        placeholder="月限额 ($)"
        value={monthly}
        onChange={(e) => setMonthly(e.target.value)}
        className="input w-24 text-xs py-1"
      />
      <input
        type="number"
        min={0}
        step={0.01}
        placeholder="日限额 ($)"
        value={daily}
        onChange={(e) => setDaily(e.target.value)}
        className="input w-24 text-xs py-1"
      />
      <Button size="sm" onClick={handleSubmit} disabled={pending || !selectedKeyId || (parseFloat(monthly) <= 0 && parseFloat(daily) <= 0)}>
        {pending ? '...' : '确认'}
      </Button>
      <Button size="sm" variant="ghost" onClick={() => setOpen(false)}>取消</Button>
    </div>
  )
}
