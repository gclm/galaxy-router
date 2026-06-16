import { useEffect, useMemo, useState } from 'react'
import type { SettingItem, InfraConfig } from '@/api/types'
import type { ImportResult, ResetResult } from '@/api/backup'
import { Button } from '@/components/ui/button'
import { ToggleSwitch } from '@/components/ToggleSwitch'
import { useAuthStore } from '@/stores/auth'
import {
  useSettings,
  useInfraConfig,
  useUpdateSetting,
  useChangePassword,
  useExportBackup,
  useImportBackup,
  useResetBackup,
} from '@/api/query-hooks'
import { toast } from 'sonner'
import { User, Shield, TrendingUp, Sliders, Server, Database, Globe, RefreshCw } from 'lucide-react'

const tabs = [
  { id: 'account', label: '账户安全', icon: Shield },
  { id: 'scheduler', label: '调度策略', icon: Sliders },
  { id: 'sticky-session', label: '粘性会话', icon: TrendingUp },
  { id: 'cors', label: '跨域设置', icon: Globe },
  { id: 'backup', label: '数据备份', icon: Database },
  { id: 'proxy', label: '上游代理', icon: Globe },
  { id: 'update', label: '版本更新', icon: RefreshCw },
  { id: 'infra', label: '基础配置', icon: Server },
] as const

type TabId = typeof tabs[number]['id']

type FieldDef = {
  key: string
  label: string
  description?: string
  type: 'switch' | 'number' | 'text' | 'select'
  options?: { value: string; label: string }[]
  unit?: string
  min?: number
  max?: number
}

const fieldDefs: Record<string, FieldDef[]> = {
  scheduler: [
    { key: 'scheduler.top_k', label: 'Top-K 候选数量', description: '每次选择时保留的最佳候选数量', type: 'number', min: 1, max: 50 },
    { key: 'scheduler.priority', label: '优先级权重', type: 'number', min: 0, max: 5, unit: '' },
    { key: 'scheduler.load', label: '负载权重', type: 'number', min: 0, max: 5, unit: '' },
    { key: 'scheduler.queue', label: '队列权重', type: 'number', min: 0, max: 5, unit: '' },
    { key: 'scheduler.error_rate', label: '错误率权重', type: 'number', min: 0, max: 5, unit: '' },
    { key: 'scheduler.ttft', label: '首 Token 时间权重', type: 'number', min: 0, max: 5, unit: '' },
  ],
  sticky_session: [
    { key: 'sticky_session.enabled', label: '启用粘性会话', description: '同一 session_hash 路由到同一上游', type: 'switch' },
    { key: 'sticky_session.ttl_seconds', label: '会话保持时间', type: 'number', min: 60, max: 86400, unit: '秒' },
  ],
  cors: [
    { key: 'cors.allow_origins', label: '允许的跨域来源', description: '逗号分隔域名列表，空=禁止跨域，*=允许所有（如 "https://example.com,http://localhost:3000"）', type: 'text' },
  ],
  proxy: [
    { key: 'proxy.enabled', label: '启用上游代理', description: '通过代理服务器转发请求到上游 API', type: 'switch' },
    { key: 'proxy.url', label: '代理地址', description: '如 http://127.0.0.1:7890', type: 'text' },
  ],
  update: [
    { key: 'github.repo', label: 'GitHub 仓库', description: 'owner/repo，用于检查版本更新', type: 'text' },
    { key: 'update.mirror', label: '下载镜像', description: 'api.github.com 失败时走镜像下载 release-info.json（如 https://ghfast.top/），留空=不启用', type: 'text' },
  ],
}

export function Settings() {
  const [activeTab, setActiveTab] = useState<TabId>('account')

  const { data: settings = [] } = useSettings()
  const { data: infra } = useInfraConfig()
  const updateSetting = useUpdateSetting()

  const settingMap = useMemo(() => Object.fromEntries(settings.map((s) => [s.key, s])), [settings])

  const getWeightValue = (weightKey: string): string => {
    const weightsStr = settingMap['scheduler.score_weights']?.value
    if (!weightsStr) return '1.0'
    try {
      const obj = JSON.parse(weightsStr)
      const field = weightKey.replace('scheduler.', '')
      return String(obj[field] ?? 1.0)
    } catch {
      return '1.0'
    }
  }

  const handleUpdate = async (key: string, value: string) => {
    try {
      await updateSetting.mutateAsync({ key, value })
      toast.success('保存成功')
    } catch (err) {
      toast.error(err instanceof Error ? err.message : '保存失败')
    }
  }

  const handleWeightUpdate = async (weightName: string, rawValue: string) => {
    const val = parseFloat(rawValue)
    if (isNaN(val)) return
    const weightsStr = settingMap['scheduler.score_weights']?.value
    let obj: Record<string, number> = { priority: 1.0, load: 1.0, queue: 0.7, error_rate: 0.8, ttft: 0.5 }
    if (weightsStr) {
      try { obj = JSON.parse(weightsStr) } catch { /* ignore */ }
    }
    obj[weightName] = val
    try {
      await updateSetting.mutateAsync({ key: 'scheduler.score_weights', value: JSON.stringify(obj) })
      toast.success('保存成功')
    } catch (err) {
      toast.error(err instanceof Error ? err.message : '保存失败')
    }
  }

  return (
    <div className="max-w-4xl space-y-4">
      <div className="border-b">
        <nav className="-mb-px flex gap-6 overflow-x-auto">
          {tabs.map((tab) => (
            <button
              key={tab.id}
              onClick={() => setActiveTab(tab.id)}
              className={`flex items-center gap-2 whitespace-nowrap border-b-2 px-1 py-3 text-sm font-medium transition-colors ${
                activeTab === tab.id
                  ? 'border-primary text-primary'
                  : 'border-transparent text-muted-foreground hover:text-foreground'
              }`}
            >
              <tab.icon className="h-4 w-4" />
              {tab.label}
            </button>
          ))}
        </nav>
      </div>

      <div className="mt-6">
        {activeTab === 'account' && <AccountTab />}
        {activeTab === 'scheduler' && (
          <SchedulerTab
            settingMap={settingMap}
            getWeightValue={getWeightValue}
            onUpdate={handleUpdate}
            onWeightUpdate={handleWeightUpdate}
          />
        )}
        {activeTab === 'sticky-session' && (
          <FieldSetTab category="sticky_session" settingMap={settingMap} onUpdate={handleUpdate} />
        )}
        {activeTab === 'cors' && (
          <CorsTab settingMap={settingMap} onUpdate={handleUpdate} />
        )}
        {activeTab === 'backup' && <BackupTab />}
        {activeTab === 'proxy' && (
          <FieldSetTab category="proxy" settingMap={settingMap} onUpdate={handleUpdate} />
        )}
        {activeTab === 'update' && (
          <FieldSetTab category="update" settingMap={settingMap} onUpdate={handleUpdate} />
        )}
        {activeTab === 'infra' && infra && <InfraTab config={infra} />}
      </div>
    </div>
  )
}

/* ── 账户安全 ── */

function AccountTab() {
  const { user } = useAuthStore()
  const [oldPassword, setOldPassword] = useState('')
  const [newPassword, setNewPassword] = useState('')
  const [confirmPassword, setConfirmPassword] = useState('')
  const changePassword = useChangePassword()

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    if (newPassword !== confirmPassword) {
      toast.error('两次输入的新密码不一致')
      return
    }
    if (newPassword.length < 8) {
      toast.error('新密码至少 8 个字符')
      return
    }
    try {
      await changePassword.mutateAsync({ old_password: oldPassword, new_password: newPassword })
      toast.success('密码修改成功')
      setOldPassword('')
      setNewPassword('')
      setConfirmPassword('')
    } catch (err) {
      toast.error(err instanceof Error ? err.message : '修改失败')
    }
  }

  return (
    <div className="space-y-6">
      <section className="rounded-2xl border bg-card p-5 space-y-3">
        <h2 className="text-sm font-medium text-muted-foreground flex items-center gap-2">
          <User className="h-4 w-4" />
          账户信息
        </h2>
        <div className="grid grid-cols-2 gap-4 text-sm">
          <div>
            <span className="text-muted-foreground">用户名</span>
            <p className="font-medium">{user?.username}</p>
          </div>
          <div>
            <span className="text-muted-foreground">用户 ID</span>
            <p><code className="rounded bg-muted px-1.5 py-0.5 text-xs">{user?.id}</code></p>
          </div>
        </div>
      </section>

      <section className="rounded-2xl border bg-card p-5 space-y-4">
        <h2 className="text-sm font-medium text-muted-foreground">修改密码</h2>
        <form onSubmit={handleSubmit} className="space-y-3">
          <div>
            <label className="block text-sm font-medium mb-1">当前密码</label>
            <input type="password" value={oldPassword} onChange={(e) => setOldPassword(e.target.value)} className="input" required />
          </div>
          <div>
            <label className="block text-sm font-medium mb-1">新密码</label>
            <input type="password" value={newPassword} onChange={(e) => setNewPassword(e.target.value)} className="input" placeholder="至少 8 个字符" required />
          </div>
          <div>
            <label className="block text-sm font-medium mb-1">确认新密码</label>
            <input type="password" value={confirmPassword} onChange={(e) => setConfirmPassword(e.target.value)} className="input" required />
          </div>
          <Button type="submit" disabled={changePassword.isPending} className="btn-primary">
            {changePassword.isPending ? '修改中...' : '修改密码'}
          </Button>
        </form>
      </section>
    </div>
  )
}

/* ── 调度策略（权重字段拆分） ── */

function SchedulerTab({
  settingMap,
  getWeightValue,
  onUpdate,
  onWeightUpdate,
}: {
  settingMap: Record<string, SettingItem>
  getWeightValue: (key: string) => string
  onUpdate: (key: string, value: string) => Promise<void>
  onWeightUpdate: (weightName: string, value: string) => Promise<void>
}) {
  const topKField = fieldDefs.scheduler.find((f) => f.key === 'scheduler.top_k')!
  const weightFields = fieldDefs.scheduler.filter((f) => f.key !== 'scheduler.top_k')

  return (
    <section className="rounded-2xl border bg-card divide-y">
      {/* Top-K */}
      <SettingRow label={topKField.label} description={topKField.description}>
        <InlineNumberEdit
          value={settingMap[topKField.key]?.value ?? '7'}
          onSave={async (v) => onUpdate(topKField.key, v)}
          min={topKField.min}
          max={topKField.max}
        />
      </SettingRow>

      {/* 权重 */}
      {weightFields.map((field) => (
        <SettingRow key={field.key} label={field.label}>
          <InlineNumberEdit
            value={getWeightValue(field.key)}
            onSave={async (v) => onWeightUpdate(field.key.replace('scheduler.', ''), v)}
            min={field.min}
            max={field.max}
            step={0.1}
          />
        </SettingRow>
      ))}
    </section>
  )
}

/* ── 通用字段标签页（粘性会话、统计日志） ── */

function renderFieldControl(field: FieldDef, value: string, onUpdate: (key: string, value: string) => Promise<void>) {
  const props = { value, onSave: (v: string) => onUpdate(field.key, v) }
  switch (field.type) {
    case 'switch':
      return <ToggleSwitch enabled={value === 'true'} onClick={() => onUpdate(field.key, value === 'true' ? 'false' : 'true')} size="md" />
    case 'select':
      return field.options ? <SelectControl {...props} options={field.options} /> : null
    case 'text':
      return <InlineTextEdit {...props} />
    default:
      return <InlineNumberEdit {...props} min={field.min} max={field.max} unit={field.unit} />
  }
}

function FieldSetTab({
  category,
  settingMap,
  onUpdate,
}: {
  category: string
  settingMap: Record<string, SettingItem>
  onUpdate: (key: string, value: string) => Promise<void>
}) {
  const fields = fieldDefs[category] ?? []

  if (fields.length === 0) {
    return (
      <section className="rounded-2xl border bg-card p-8 text-center">
        <p className="text-sm text-muted-foreground">暂无配置项</p>
      </section>
    )
  }

  return (
    <section className="rounded-2xl border bg-card divide-y">
      {fields.map((field) => (
        <SettingRow key={field.key} label={field.label} description={field.description}>
          {renderFieldControl(field, settingMap[field.key]?.value ?? '', onUpdate)}
        </SettingRow>
      ))}
    </section>
  )
}

/* ── 基础配置（只读） ── */

function InfraTab({ config }: { config: InfraConfig }) {
  const sections = [
    {
      title: '服务器',
      items: [
        { label: '监听地址', value: config.server.host },
        { label: '端口', value: String(config.server.port) },
      ],
    },
    {
      title: '数据库',
      items: [{ label: '路径', value: config.database.path }],
    },
    {
      title: '日志',
      items: [
        { label: '级别', value: config.logging.level },
        { label: '格式', value: config.logging.format },
        { label: '输出到文件', value: config.logging.file ? '是' : '否' },
        { label: '文件路径', value: config.logging.file_path },
      ],
    },
    {
      title: '认证',
      items: [{ label: 'Token 过期时间', value: `${config.auth.token_expiry_hours} 小时` }],
    },
  ]

  return (
    <div className="space-y-4">
      {sections.map((section) => (
        <section key={section.title} className="rounded-2xl border bg-card p-5 space-y-3">
          <h2 className="text-sm font-medium text-muted-foreground">{section.title}</h2>
          <div className="grid grid-cols-2 gap-3 text-sm">
            {section.items.map((item) => (
              <div key={item.label}>
                <span className="text-muted-foreground">{item.label}</span>
                <p className="font-medium"><code className="rounded bg-muted px-1.5 py-0.5 text-xs">{item.value}</code></p>
              </div>
            ))}
          </div>
        </section>
      ))}
      <p className="text-xs text-muted-foreground">以上配置来自 config.toml，修改后需重启生效。</p>
    </div>
  )
}

/* ── 通用行组件 ── */

function SettingRow({ label, description, children }: { label: string; description?: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-4 px-5 py-4">
      <div className="min-w-0">
        <p className="text-sm font-medium">{label}</p>
        {description && <p className="text-xs text-muted-foreground mt-0.5">{description}</p>}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  )
}

/* ── Select 控件 ── */

function SelectControl({ value, options, onSave }: { value: string; options: { value: string; label: string }[]; onSave: (v: string) => Promise<void> }) {
  const [pending, setPending] = useState(false)
  const handleChange = async (e: React.ChangeEvent<HTMLSelectElement>) => {
    setPending(true)
    try { await onSave(e.target.value) } finally { setPending(false) }
  }
  return (
    <select
      value={value}
      onChange={handleChange}
      disabled={pending}
      className="input w-auto min-w-[140px]"
    >
      {options.map((opt) => (
        <option key={opt.value} value={opt.value}>{opt.label}</option>
      ))}
    </select>
  )
}

/* ── 内联文本编辑控件 ── */

function InlineTextEdit({ value, onSave }: { value: string; onSave: (v: string) => Promise<void> }) {
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState(value)
  const [pending, setPending] = useState(false)

  const save = async () => {
    setPending(true)
    try {
      await onSave(draft)
      setEditing(false)
    } finally {
      setPending(false)
    }
  }

  if (!editing) {
    return (
      <button
        type="button"
        onClick={() => { setDraft(value); setEditing(true) }}
        className="flex items-center gap-1 rounded-lg bg-muted px-3 py-1.5 text-sm font-medium hover:bg-muted/80 transition-colors max-w-[200px] truncate"
      >
        {value || '未设置'}
      </button>
    )
  }

  return (
    <div className="flex items-center gap-2">
      <input
        type="text"
        className="input w-60"
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => { if (e.key === 'Enter') save(); if (e.key === 'Escape') setEditing(false) }}
        autoFocus
        disabled={pending}
      />
      <Button size="sm" onClick={save} disabled={pending}>保存</Button>
      <Button size="sm" variant="ghost" onClick={() => setEditing(false)}>取消</Button>
    </div>
  )
}

/* ── 内联数字编辑控件 ── */

function InlineNumberEdit({ value, onSave, min, max, step, unit }: {
  value: string
  onSave: (v: string) => Promise<void>
  min?: number
  max?: number
  step?: number
  unit?: string
}) {
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState(value)
  const [pending, setPending] = useState(false)

  const save = async () => {
    const num = parseFloat(draft)
    if (isNaN(num)) return
    if (min !== undefined && num < min) return
    if (max !== undefined && num > max) return
    setPending(true)
    try {
      await onSave(String(num))
      setEditing(false)
    } finally {
      setPending(false)
    }
  }

  if (!editing) {
    return (
      <button
        type="button"
        onClick={() => { setDraft(value); setEditing(true) }}
        className="flex items-center gap-1 rounded-lg bg-muted px-3 py-1.5 text-sm font-medium hover:bg-muted/80 transition-colors"
      >
        {value}
        {unit && <span className="text-muted-foreground">{unit}</span>}
      </button>
    )
  }

  return (
    <div className="flex items-center gap-2">
      <input
        type="number"
        className="input w-24"
        value={draft}
        min={min}
        max={max}
        step={step ?? 1}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => { if (e.key === 'Enter') save(); if (e.key === 'Escape') setEditing(false) }}
        autoFocus
        disabled={pending}
      />
      {unit && <span className="text-xs text-muted-foreground">{unit}</span>}
      <Button size="sm" onClick={save} disabled={pending}>保存</Button>
      <Button size="sm" variant="ghost" onClick={() => setEditing(false)}>取消</Button>
    </div>
  )
}

/* ── 跨域设置（标签化输入） ── */

function CorsTab({
  settingMap,
  onUpdate,
}: {
  settingMap: Record<string, SettingItem>
  onUpdate: (key: string, value: string) => Promise<void>
}) {
  const corsValue = settingMap['cors.allow_origins']?.value ?? ''
  const [inputValue, setInputValue] = useState('')
  const [pending, setPending] = useState(false)
  const [savedValue, setSavedValue] = useState(corsValue)

  useEffect(() => {
    setSavedValue(corsValue)
  }, [corsValue])

  const originsList = useMemo(() => {
    const value = savedValue.trim()
    if (!value) return []
    if (value === '*') return ['*']
    return Array.from(new Set(
      value.split(/[,，]/).map(item => item.trim()).filter(Boolean)
    ))
  }, [savedValue])

  const handleAdd = async () => {
    const raw = inputValue.trim()
    if (!raw) return
    const newOrigins = raw.split(/[,，]/).map(s => s.trim()).filter(Boolean)
    if (newOrigins.includes('*')) {
      await saveOrigins(['*'])
    } else {
      await saveOrigins([...originsList, ...newOrigins])
    }
    setInputValue('')
  }

  const handleRemove = async (origin: string) => {
    const next = originsList.filter(o => o !== origin)
    await saveOrigins(next)
  }

  const handlePreset = async (preset: string) => {
    await saveOrigins(preset === '*' ? ['*'] : [])
  }

  const saveOrigins = async (origins: string[]) => {
    const normalized = Array.from(new Set(origins.map(o => o.trim()).filter(Boolean)))
    const value = normalized.includes('*') ? '*' : normalized.join(',')
    setPending(true)
    try {
      await onUpdate('cors.allow_origins', value)
      setSavedValue(value)
    } catch (err) {
      toast.error(err instanceof Error ? err.message : '保存失败')
    } finally {
      setPending(false)
    }
  }

  return (
    <section className="rounded-2xl border bg-card p-5 space-y-4">
      <h2 className="text-sm font-medium text-muted-foreground flex items-center gap-2">
        <Globe className="h-4 w-4" />
        跨域白名单
      </h2>

      <p className="text-xs text-muted-foreground">
        控制哪些域名可以通过浏览器访问本服务的 API。
        为空时禁止所有跨域请求，<code className="rounded bg-muted px-1">{'*'}</code> 表示允许所有来源。
      </p>

      {/* 快捷操作 */}
      <div className="flex gap-2">
        <Button
          size="sm"
          variant={savedValue === '*' ? 'default' : 'outline'}
          onClick={() => handlePreset('*')}
          disabled={pending}
        >
          允许所有 ({'*'})
        </Button>
        <Button
          size="sm"
          variant={savedValue === '' ? 'default' : 'outline'}
          onClick={() => handlePreset('')}
          disabled={pending}
        >
          禁止跨域
        </Button>
      </div>

      {/* 已添加的域名标签 */}
      {originsList.length > 0 && (
        <div className="flex flex-wrap gap-2">
          {originsList.map((origin) => (
            <span
              key={origin}
              className="inline-flex items-center gap-1 rounded-full bg-muted px-3 py-1 text-xs font-medium"
            >
              {origin === '*' ? '所有来源' : origin}
              <button
                type="button"
                onClick={() => handleRemove(origin)}
                className="text-muted-foreground hover:text-foreground transition-colors"
                disabled={pending}
              >
                ×
              </button>
            </span>
          ))}
        </div>
      )}

      {/* 输入框 */}
      <div className="flex gap-2">
        <input
          type="text"
          className="input flex-1"
          placeholder="输入域名，如 https://example.com 或多个用逗号分隔"
          value={inputValue}
          onChange={(e) => setInputValue(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') handleAdd() }}
          disabled={pending}
        />
        <Button size="sm" onClick={handleAdd} disabled={pending || !inputValue.trim()}>
          添加
        </Button>
      </div>
    </section>
  )
}

/* ── 数据备份 ── */

function BackupTab() {
  const [importResult, setImportResult] = useState<ImportResult | null>(null)
  const [importError, setImportError] = useState('')
  const [resetResult, setResetResult] = useState<ResetResult | null>(null)
  const [showResetConfirm, setShowResetConfirm] = useState(false)

  const exportBackup = useExportBackup()
  const importBackup = useImportBackup()
  const resetBackup = useResetBackup()

  const handleExport = async () => {
    exportBackup.reset()
    try {
      const data = await exportBackup.mutateAsync()
      const blob = new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' })
      const url = URL.createObjectURL(blob)
      const a = document.createElement('a')
      const date = new Date().toISOString().slice(0, 10)
      a.href = url
      a.download = `galaxy-router-backup-${date}.json`
      a.click()
      URL.revokeObjectURL(url)
      toast.success('导出成功')
    } catch (err) {
      toast.error(err instanceof Error ? err.message : '导出失败')
    }
  }

  const handleImport = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    if (!file) return
    setImportResult(null)
    setImportError('')

    try {
      const text = await file.text()
      const data = JSON.parse(text)
      const result = await importBackup.mutateAsync(data)
      setImportResult(result)
      toast.success('导入成功')
    } catch (err) {
      setImportError(err instanceof Error ? err.message : '导入失败')
      toast.error(err instanceof Error ? err.message : '导入失败')
    } finally {
      e.target.value = ''
    }
  }

  const handleReset = async () => {
    resetBackup.reset()
    setShowResetConfirm(false)
    try {
      const result = await resetBackup.mutateAsync()
      setResetResult(result)
      toast.success('已恢复出厂设置')
    } catch (err) {
      toast.error(err instanceof Error ? err.message : '重置失败')
    }
  }

  return (
    <div className="space-y-4">
      <section className="rounded-2xl border bg-card p-5 space-y-4">
        <h2 className="text-sm font-medium text-muted-foreground flex items-center gap-2">
          <Database className="h-4 w-4" />
          导出数据
        </h2>
        <p className="text-sm text-muted-foreground">
          导出当前系统的渠道、分组、API Key 和设置配置为 JSON 文件。
        </p>
        <p className="text-xs text-amber-600">
          备份文件包含上游 API Key 明文，请妥善保管。
        </p>
        <Button onClick={handleExport} disabled={exportBackup.isPending} className="btn-primary">
          {exportBackup.isPending ? '导出中...' : '导出备份'}
        </Button>
      </section>

      <section className="rounded-2xl border bg-card p-5 space-y-4">
        <h2 className="text-sm font-medium text-muted-foreground flex items-center gap-2">
          <Database className="h-4 w-4" />
          导入数据
        </h2>
        <p className="text-sm text-muted-foreground">
          从备份文件恢复配置。同名渠道、分组和 API Key 将被跳过，设置项将更新。
        </p>
        <div>
          <label className={`inline-flex cursor-pointer items-center gap-2 rounded-lg px-4 py-2 text-sm font-medium transition-colors ${
            importBackup.isPending
              ? 'bg-muted text-muted-foreground cursor-not-allowed'
              : 'bg-primary text-primary-foreground hover:bg-primary/90'
          }`}>
            {importBackup.isPending ? '导入中...' : '选择备份文件'}
            <input
              type="file"
              accept=".json"
              onChange={handleImport}
              disabled={importBackup.isPending}
              className="hidden"
            />
          </label>
        </div>
        {importError && (
          <div className="rounded-lg bg-destructive/10 p-3 text-sm text-destructive">{importError}</div>
        )}
        {importResult && (
          <div className="rounded-lg bg-muted p-4 text-sm space-y-2">
            <p className="font-medium">导入结果</p>
            <div className="grid grid-cols-2 gap-2 text-muted-foreground">
              <span>渠道: {importResult.channels_imported}</span>
              <span>分组: {importResult.groups_imported}</span>
              <span>API Key: {importResult.api_keys_imported}</span>
              <span>设置: {importResult.settings_imported}</span>
            </div>
            {importResult.errors.length > 0 && (
              <div className="mt-2 space-y-1">
                {importResult.errors.map((err, i) => (
                  <p key={i} className="text-xs text-amber-600">{err}</p>
                ))}
              </div>
            )}
          </div>
        )}
      </section>

      <section className="rounded-2xl border border-destructive/30 bg-card p-5 space-y-4">
        <h2 className="text-sm font-medium text-destructive flex items-center gap-2">
          <Database className="h-4 w-4" />
          恢复出厂设置
        </h2>
        <p className="text-sm text-muted-foreground">
          清空所有渠道、分组、API Key 和设置数据，恢复为出厂状态。管理员账户和定价数据不受影响。
        </p>
        <p className="text-xs text-amber-600">
          此操作不可撤销，建议先导出备份。
        </p>
        {showResetConfirm ? (
          <div className="flex items-center gap-3">
            <Button onClick={handleReset} variant="destructive" disabled={resetBackup.isPending}>
              {resetBackup.isPending ? '重置中...' : '确认重置'}
            </Button>
            <Button variant="outline" onClick={() => setShowResetConfirm(false)}>取消</Button>
          </div>
        ) : (
          <Button variant="destructive" onClick={() => { setShowResetConfirm(true); setResetResult(null) }}>
            恢复出厂设置
          </Button>
        )}
        {resetResult && (
          <div className="rounded-lg bg-muted p-4 text-sm space-y-2">
            <p className="font-medium text-destructive">已重置</p>
            <div className="grid grid-cols-2 gap-2 text-muted-foreground">
              <span>删除渠道: {resetResult.channels_deleted}</span>
              <span>删除分组: {resetResult.groups_deleted}</span>
              <span>删除 API Key: {resetResult.api_keys_deleted}</span>
              <span>重置设置: {resetResult.settings_reset}</span>
            </div>
          </div>
        )}
      </section>
    </div>
  )
}
