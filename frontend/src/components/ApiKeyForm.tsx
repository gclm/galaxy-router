import { useState } from 'react'
import type { ApiKey, BudgetLimit, CreateApiKeyRequest, Group } from '@/api/types'
import { Button } from '@/components/ui/button'
import { Check, Copy } from 'lucide-react'

interface ApiKeyFormProps {
  /** 传入则为编辑模式，否则为创建模式 */
  apiKey?: ApiKey
  /** 可选的 budget（编辑模式） */
  budget?: BudgetLimit
  /** 可用分组列表 */
  groups: Group[]
  /** 提交回调（返回值仅创建模式使用：新创建的 key 对象） */
  onSubmit: (data: CreateApiKeyRequest & { budget_monthly?: number; budget_daily?: number }) => void
  onCancel: () => void
  /** 创建成功后的 key 展示（仅创建模式） */
  newKeyResult?: ApiKey | null
  /** 复制回调 */
  onCopy: (key: string) => void
  /** 当前已复制的 key */
  copiedKey: string | null
  /** 按钮加载态 */
  submitting: boolean
}

export function ApiKeyForm({
  apiKey,
  budget,
  groups,
  onSubmit,
  onCancel,
  newKeyResult,
  onCopy,
  copiedKey,
  submitting,
}: ApiKeyFormProps) {
  const [name, setName] = useState(apiKey?.name ?? '')
  const [rateLimitRpm, setRateLimitRpm] = useState(
    apiKey && apiKey.rate_limit_rpm > 0 ? String(apiKey.rate_limit_rpm) : '',
  )
  const [rateLimitTpm, setRateLimitTpm] = useState(
    apiKey && apiKey.rate_limit_tpm > 0 ? String(apiKey.rate_limit_tpm) : '',
  )
  const [selectedGroups, setSelectedGroups] = useState<string[]>(
    apiKey?.allowed_groups
      ? apiKey.allowed_groups.split(',').map((s) => s.trim()).filter(Boolean)
      : [],
  )
  const [budgetMonthly, setBudgetMonthly] = useState(
    budget?.monthly_limit_usd ? String(budget.monthly_limit_usd) : '',
  )
  const [budgetDaily, setBudgetDaily] = useState(
    budget?.daily_limit_usd ? String(budget.daily_limit_usd) : '',
  )
  const [groupSearch, setGroupSearch] = useState('')

  const toggleGroup = (group: string) => {
    setSelectedGroups((prev) =>
      prev.includes(group) ? prev.filter((g) => g !== group) : [...prev, group],
    )
  }

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    if (!name.trim()) return

    const data: CreateApiKeyRequest & { budget_monthly?: number; budget_daily?: number } = {
      name: name.trim(),
      rate_limit_rpm: rateLimitRpm ? parseInt(rateLimitRpm) : undefined,
      rate_limit_tpm: rateLimitTpm ? parseInt(rateLimitTpm) : undefined,
      allowed_groups: selectedGroups.length > 0 ? selectedGroups.join(',') : undefined,
    }

    // 附带预算(创建/编辑都支持)
    const monthly = budgetMonthly ? parseFloat(budgetMonthly) : 0
    const daily = budgetDaily ? parseFloat(budgetDaily) : 0
    if (monthly > 0 || daily > 0) {
      data.budget_monthly = monthly
      data.budget_daily = daily
    }

    onSubmit(data)
  }

  // 创建成功 → 展示 key
  if (newKeyResult) {
    return (
      <div className="space-y-4">
        <div className="rounded-lg bg-green-50 border border-green-200 p-3 dark:bg-green-900/20 dark:border-green-800">
          <p className="text-sm text-green-800 dark:text-green-400 mb-3">
            API Key 已创建，请妥善保存
          </p>
          <div className="flex items-center gap-2">
            <code className="flex-1 rounded bg-background px-3 py-2 text-sm font-mono break-all">
              {newKeyResult.api_key}
            </code>
            <Button
              variant="outline"
              size="sm"
              onClick={() => onCopy(newKeyResult.api_key)}
            >
              {copiedKey === newKeyResult.api_key ? (
                <Check className="h-4 w-4 mr-1" />
              ) : (
                <Copy className="h-4 w-4 mr-1" />
              )}
              {copiedKey === newKeyResult.api_key ? '已复制' : '复制'}
            </Button>
          </div>
        </div>
        <div className="flex justify-end">
          <Button variant="outline" onClick={onCancel}>
            我已保存
          </Button>
        </div>
      </div>
    )
  }

  return (
    <form onSubmit={handleSubmit} className="space-y-4">
      <div>
        <label className="block text-sm font-medium mb-1">名称</label>
        <input
          type="text"
          value={name}
          onChange={(e) => setName(e.target.value)}
          className="input"
          placeholder="例如：前端应用"
          onKeyDown={(e) => e.key === 'Enter' && handleSubmit(e)}
        />
      </div>

      {/* Group Selector */}
      <div>
        <label className="block text-sm font-medium mb-2">
          允许访问的分组（留空表示全部）
        </label>
        {groups.length === 0 ? (
          <p className="text-xs text-muted-foreground">暂无可用分组</p>
        ) : (
          <>
            <input
              type="text"
              value={groupSearch}
              onChange={(e) => setGroupSearch(e.target.value)}
              className="input mb-2"
              placeholder="搜索分组..."
            />
            <div className="max-h-48 overflow-y-auto rounded-lg border p-2 space-y-1">
              {groups
                .filter((g) => !groupSearch || g.name.toLowerCase().includes(groupSearch.toLowerCase()))
                .map((group) => (
                  <label
                    key={group.id}
                    className="flex items-center gap-2 px-2 py-1.5 rounded hover:bg-muted/50 cursor-pointer"
                  >
                    <input
                      type="checkbox"
                      checked={selectedGroups.includes(group.name)}
                      onChange={() => toggleGroup(group.name)}
                      className="rounded"
                    />
                    <span className="text-sm">{group.name}</span>
                  </label>
                ))}
            </div>
          </>
        )}
        {selectedGroups.length > 0 && (
          <p className="text-xs text-muted-foreground mt-1">
            已选择 {selectedGroups.length} 个分组
          </p>
        )}
      </div>

      {/* Rate Limit */}
      <div>
        <label className="block text-sm font-medium mb-2">速率限制</label>
        <div className="grid grid-cols-2 gap-3">
          <div>
            <label className="block text-xs text-muted-foreground mb-1">
              RPM（请求/分钟）
            </label>
            <input
              type="number"
              value={rateLimitRpm}
              onChange={(e) => setRateLimitRpm(e.target.value)}
              className="input"
              placeholder="不限"
            />
          </div>
          <div>
            <label className="block text-xs text-muted-foreground mb-1">
              TPM（令牌/分钟）
            </label>
            <input
              type="number"
              value={rateLimitTpm}
              onChange={(e) => setRateLimitTpm(e.target.value)}
              className="input"
              placeholder="不限"
            />
          </div>
        </div>
      </div>

      {/* Budget（创建/编辑都支持） */}
      <div>
        <label className="block text-sm font-medium mb-2">预算限制 (USD)</label>
        <div className="grid grid-cols-2 gap-3">
          <div>
            <label className="block text-xs text-muted-foreground mb-1">月限额 ($)</label>
            <input
              type="number"
              value={budgetMonthly}
              onChange={(e) => setBudgetMonthly(e.target.value)}
              className="input"
              placeholder="不限"
              step="0.01"
              min="0"
            />
          </div>
          <div>
            <label className="block text-xs text-muted-foreground mb-1">日限额 ($)</label>
            <input
              type="number"
              value={budgetDaily}
              onChange={(e) => setBudgetDaily(e.target.value)}
              className="input"
              placeholder="不限"
              step="0.01"
              min="0"
            />
          </div>
        </div>
        <p className="text-xs text-muted-foreground mt-1">留空表示不限制，超出后请求将返回 402</p>
      </div>

      {/* Buttons */}
      <div className="flex justify-end gap-2">
        <Button type="button" variant="outline" onClick={onCancel}>
          取消
        </Button>
        <Button
          type="submit"
          disabled={!name.trim() || submitting}
          className="btn-primary"
        >
          {submitting ? '保存中...' : apiKey ? '保存' : '创建'}
        </Button>
      </div>
    </form>
  )
}
