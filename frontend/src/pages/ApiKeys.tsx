import { useEffect, useRef, useState } from 'react'
import { apiKeysApi, groupsApi } from '@/api'
import type { ApiKey, Group } from '@/api/types'
import { Button } from '@/components/ui/button'
import { StatusBadge } from '@/components/StatusBadge'
import { ConfirmDeleteDialog } from '@/components/ConfirmDeleteDialog'
import { useDebouncedValue } from '@/lib/hooks'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { formatDate } from '@/lib/utils'
import {
  Plus,
  Trash2,
  Copy,
  Check,
  Search,
  RefreshCw,
} from 'lucide-react'

export function ApiKeys() {
  const [apiKeys, setApiKeys] = useState<ApiKey[]>([])
  const [loading, setLoading] = useState(true)

  const [searchInput, setSearchInput] = useState('')
  const search = useDebouncedValue(searchInput)

  const [createOpen, setCreateOpen] = useState(false)
  const [newKeyName, setNewKeyName] = useState('')
  const [newKeyResult, setNewKeyResult] = useState<ApiKey | null>(null)
  const [creating, setCreating] = useState(false)
  const [copiedKey, setCopiedKey] = useState<string | null>(null)

  const [deleteId, setDeleteId] = useState<string | null>(null)

  // 分组列表（用于模型选择）
  const [availableGroups, setAvailableGroups] = useState<Group[]>([])
  const [rateLimitRpm, setRateLimitRpm] = useState('')
  const [rateLimitTpm, setRateLimitTpm] = useState('')
  const [selectedGroups, setSelectedGroups] = useState<string[]>([])
  const fetchRef = useRef<() => void>(() => {})

  useEffect(() => {
    const controller = new AbortController()
    const run = async () => {
      setLoading(true)
      try {
        const data = await apiKeysApi.list()
        if (!controller.signal.aborted) setApiKeys(data)
      } catch (error) {
        if (!controller.signal.aborted) console.error('Failed to fetch API keys:', error)
      } finally {
        if (!controller.signal.aborted) setLoading(false)
      }
    }
    run()
    fetchRef.current = run
    return () => { controller.abort() }
  }, [])

  useEffect(() => {
    groupsApi.list({ page_size: 1000 }).then(res => setAvailableGroups(res.items)).catch(console.error)
  }, [])

  const handleCreate = async () => {
    if (!newKeyName.trim()) return
    setCreating(true)
    try {
      const allowedGroups = selectedGroups.length > 0 ? selectedGroups.join(',') : undefined
      const key = await apiKeysApi.create({
        name: newKeyName.trim(),
        allowed_groups: allowedGroups,
        rate_limit_rpm: rateLimitRpm ? parseInt(rateLimitRpm) : undefined,
        rate_limit_tpm: rateLimitTpm ? parseInt(rateLimitTpm) : undefined,
      })
      setNewKeyResult(key)
      setApiKeys(prev => [key, ...prev])
      setNewKeyName('')
      setSelectedGroups([])
      setRateLimitRpm('')
      setRateLimitTpm('')
    } catch (error) {
      console.error('Failed to create API key:', error)
    } finally {
      setCreating(false)
    }
  }

  const handleDelete = async () => {
    if (!deleteId) return
    await apiKeysApi.delete(deleteId)
    setDeleteId(null)
    fetchRef.current()
  }

  const handleToggleEnabled = async (key: ApiKey) => {
    await apiKeysApi.update(key.id, { enabled: !key.enabled })
    fetchRef.current()
  }

  const copyToClipboard = async (key: string) => {
    await navigator.clipboard.writeText(key)
    setCopiedKey(key)
    setTimeout(() => setCopiedKey(prev => prev === key ? null : prev), 2000)
  }

  const closeCreateDialog = () => {
    setCreateOpen(false)
    setNewKeyResult(null)
    setNewKeyName('')
    setSelectedGroups([])
    setRateLimitRpm('')
    setRateLimitTpm('')
  }

  const toggleGroup = (group: string) => {
    setSelectedGroups(prev =>
      prev.includes(group)
        ? prev.filter(g => g !== group)
        : [...prev, group]
    )
  }

  const filteredKeys = apiKeys.filter(k =>
    !search || k.name.toLowerCase().includes(search.toLowerCase())
  )

  const maskKey = (key: string) => {
    if (key.length <= 12) return key
    return key.substring(0, 8) + '...' + key.substring(key.length - 4)
  }

  const formatSupportedModels = (models: string | null) => {
    if (!models) return '全部模型'
    const list = models.split(',').map(s => s.trim()).filter(Boolean)
    if (list.length === 0) return '全部模型'
    if (list.length <= 3) return list.join(', ')
    return `${list.slice(0, 3).join(', ')} 等 ${list.length} 个`
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <p className="text-sm text-muted-foreground">管理客户端访问密钥</p>
        </div>
        <Button onClick={() => setCreateOpen(true)} className="btn-primary">
          <Plus className="mr-2 h-4 w-4" />
          创建 API Key
        </Button>
      </div>

      {/* 筛选栏 */}
      <div className="flex items-center gap-3">
        <div className="relative flex-1 max-w-sm">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
          <input
            type="text"
            value={searchInput}
            onChange={(e) => setSearchInput(e.target.value)}
            placeholder="搜索 Key 名称..."
            className="input pl-9"
          />
        </div>
        <Button variant="outline" size="icon" onClick={() => fetchRef.current()} title="刷新">
          <RefreshCw className="h-4 w-4" />
        </Button>
      </div>

      {/* 表格 */}
      <div className="rounded-2xl border bg-card overflow-hidden">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b bg-muted/50">
              <th className="text-left px-4 py-3 font-medium">名称</th>
              <th className="text-left px-4 py-3 font-medium">Key</th>
              <th className="text-left px-4 py-3 font-medium">分组权限</th>
              <th className="text-left px-4 py-3 font-medium">速率限制</th>
              <th className="text-center px-4 py-3 font-medium">状态</th>
              <th className="text-left px-4 py-3 font-medium">创建时间</th>
              <th className="text-center px-4 py-3 font-medium">操作</th>
            </tr>
          </thead>
          <tbody>
            {loading ? (
              <tr>
                <td colSpan={7} className="text-center py-12 text-muted-foreground">加载中...</td>
              </tr>
            ) : filteredKeys.length === 0 ? (
              <tr>
                <td colSpan={7} className="text-center py-12 text-muted-foreground">
                  {search ? '没有匹配的 API Key' : '暂无 API Key，点击上方按钮创建'}
                </td>
              </tr>
            ) : (
              filteredKeys.map((apiKey) => (
                <tr key={apiKey.id} className="border-b last:border-0 hover:bg-muted/30 transition-colors">
                  <td className="px-4 py-3 font-medium">{apiKey.name}</td>
                  <td className="px-4 py-3">
                    <div className="flex items-center gap-2">
                      <code className="rounded bg-muted px-2 py-0.5 text-xs font-mono">
                        {maskKey(apiKey.api_key)}
                      </code>
                      <button
                        onClick={() => copyToClipboard(apiKey.api_key)}
                        className={`transition-colors ${
                          copiedKey === apiKey.api_key
                            ? 'text-green-500'
                            : 'text-muted-foreground hover:text-foreground'
                        }`}
                        title="复制完整 Key"
                      >
                        {copiedKey === apiKey.api_key
                          ? <Check className="h-3.5 w-3.5" />
                          : <Copy className="h-3.5 w-3.5" />}
                      </button>
                    </div>
                  </td>
                  <td className="px-4 py-3 text-xs text-muted-foreground">
                    {apiKey.allowed_groups ? formatSupportedModels(apiKey.allowed_groups) : '全部'}
                  </td>
                  <td className="px-4 py-3 text-xs text-muted-foreground">
                    {apiKey.rate_limit_rpm > 0 || apiKey.rate_limit_tpm > 0
                      ? `${apiKey.rate_limit_rpm || '∞'} RPM / ${apiKey.rate_limit_tpm || '∞'} TPM`
                      : '不限'}
                  </td>
                  <td className="px-4 py-3 text-center">
                    <StatusBadge enabled={apiKey.enabled} onClick={() => handleToggleEnabled(apiKey)} />
                  </td>
                  <td className="px-4 py-3 text-muted-foreground text-xs">{formatDate(apiKey.created_at)}</td>
                  <td className="px-4 py-3">
                    <div className="flex items-center justify-center">
                      <Button variant="ghost" size="icon" className="h-8 w-8 text-destructive hover:text-destructive" onClick={() => setDeleteId(apiKey.id)} title="删除">
                        <Trash2 className="h-3.5 w-3.5" />
                      </Button>
                    </div>
                  </td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>

      {/* 创建 API Key Dialog */}
      <Dialog open={createOpen} onOpenChange={(open) => { if (!open) closeCreateDialog() }}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>{newKeyResult ? 'API Key 创建成功' : '创建 API Key'}</DialogTitle>
          </DialogHeader>
          {!newKeyResult ? (
            <div className="space-y-4">
              <div>
                <label className="block text-sm font-medium mb-1">名称</label>
                <input
                  type="text"
                  value={newKeyName}
                  onChange={(e) => setNewKeyName(e.target.value)}
                  className="input"
                  placeholder="例如：前端应用"
                  onKeyDown={(e) => e.key === 'Enter' && handleCreate()}
                />
              </div>
              <div>
                <label className="block text-sm font-medium mb-2">允许访问的分组（留空表示全部）</label>
                {availableGroups.length === 0 ? (
                  <p className="text-xs text-muted-foreground">暂无可用分组</p>
                ) : (
                  <div className="max-h-48 overflow-y-auto rounded-lg border p-2 space-y-1">
                    {availableGroups.map(group => (
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
                )}
                {selectedGroups.length > 0 && (
                  <p className="text-xs text-muted-foreground mt-1">
                    已选择 {selectedGroups.length} 个分组
                  </p>
                )}
              </div>
              <div>
                <label className="block text-sm font-medium mb-2">速率限制</label>
                <div className="grid grid-cols-2 gap-3">
                  <div>
                    <label className="block text-xs text-muted-foreground mb-1">RPM（请求/分钟）</label>
                    <input
                      type="number"
                      value={rateLimitRpm}
                      onChange={(e) => setRateLimitRpm(e.target.value)}
                      className="input"
                      placeholder="不限"
                    />
                  </div>
                  <div>
                    <label className="block text-xs text-muted-foreground mb-1">TPM（令牌/分钟）</label>
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
              <div className="flex justify-end gap-2">
                <Button variant="outline" onClick={closeCreateDialog}>取消</Button>
                <Button onClick={handleCreate} disabled={!newKeyName.trim() || creating} className="btn-primary">
                  {creating ? '创建中...' : '创建'}
                </Button>
              </div>
            </div>
          ) : (
            <div className="space-y-4">
              <div className="rounded-lg bg-green-50 border border-green-200 p-3 dark:bg-green-900/20 dark:border-green-800">
                <p className="text-sm text-green-800 dark:text-green-400 mb-3">
                  API Key 已创建，可随时在列表中复制
                </p>
                <div className="flex items-center gap-2">
                  <code className="flex-1 rounded bg-background px-3 py-2 text-sm font-mono break-all">
                    {newKeyResult.api_key}
                  </code>
                  <Button variant="outline" size="sm" onClick={() => copyToClipboard(newKeyResult.api_key)}>
                    {copiedKey === newKeyResult.api_key ? <Check className="h-4 w-4 mr-1" /> : <Copy className="h-4 w-4 mr-1" />}
                    {copiedKey === newKeyResult.api_key ? '已复制' : '复制'}
                  </Button>
                </div>
              </div>
              <div className="flex justify-end">
                <Button variant="outline" onClick={closeCreateDialog}>我已保存</Button>
              </div>
            </div>
          )}
        </DialogContent>
      </Dialog>

      {/* 删除确认 Dialog */}
      <ConfirmDeleteDialog
        open={!!deleteId}
        onOpenChange={(open) => { if (!open) setDeleteId(null) }}
        message="确定要删除此 API Key 吗？使用该 Key 的应用将无法继续访问。"
        onConfirm={handleDelete}
      />
    </div>
  )
}
