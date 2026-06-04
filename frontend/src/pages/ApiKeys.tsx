import { useState, useMemo } from 'react'
import { apiKeysApi } from '@/api/api-keys'
import type { ApiKey, CreateApiKeyRequest } from '@/api/types'
import { Button } from '@/components/ui/button'
import { StatusBadge } from '@/components/StatusBadge'
import { ConfirmDeleteDialog } from '@/components/ConfirmDeleteDialog'
import { FilterBar } from '@/components/common/FilterBar'
import { PageHeader } from '@/components/common/PageHeader'
import { EmptyState } from '@/components/common/EmptyState'
import { useTableLoader } from '@/hooks/useTableLoader'
import {
  useCreateApiKey,
  useUpdateApiKey,
  useDeleteApiKey,
  useGroups,
} from '@/api/query-hooks'
import { formatDate, maskKey } from '@/lib/utils'
import { toast } from 'sonner'
import {
  Plus,
  Pencil,
  Trash2,
  Copy,
  Check,
} from 'lucide-react'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'

export function ApiKeys() {
  // ─── Table state via useTableLoader ──────────────────────
  const table = useTableLoader<ApiKey>({
    fetchFn: async () => {
      const result = await apiKeysApi.list()
      return { items: result, total: result.length }
    },
    defaultPageSize: 20,
  })

  // ─── Client-side filter + sort + paginate ───────────────
  const displayData = useMemo(() => {
    let items = [...table.data]

    if (table.search) {
      const q = table.search.toLowerCase()
      items = items.filter((k) => k.name.toLowerCase().includes(q))
    }

    if (table.status === 'enabled') {
      items = items.filter((k) => k.enabled)
    } else if (table.status === 'disabled') {
      items = items.filter((k) => !k.enabled)
    }

    items.sort((a, b) => {
      const av =
        table.sortBy === 'name'
          ? a.name.toLowerCase()
          : a.created_at.toLowerCase()
      const bv =
        table.sortBy === 'name'
          ? b.name.toLowerCase()
          : b.created_at.toLowerCase()
      if (av < bv) return table.sortOrder === 'asc' ? -1 : 1
      if (av > bv) return table.sortOrder === 'asc' ? 1 : -1
      return 0
    })

    const total = items.length
    const start = (table.page - 1) * table.pageSize
    return { items: items.slice(start, start + table.pageSize), total }
  }, [
    table.data,
    table.search,
    table.status,
    table.sortBy,
    table.sortOrder,
    table.page,
    table.pageSize,
  ])

  // ─── Dialog state ───────────────────────────────────────
  const [formOpen, setFormOpen] = useState(false)
  const [editingKey, setEditingKey] = useState<ApiKey | null>(null)
  const [deleteId, setDeleteId] = useState<string | null>(null)
  const [copiedKey, setCopiedKey] = useState<string | null>(null)

  // ─── Create form state ──────────────────────────────────
  const [newKeyName, setNewKeyName] = useState('')
  const [newKeySupportedModels, setNewKeySupportedModels] = useState('')
  const [newKeyRateLimitRpm, setNewKeyRateLimitRpm] = useState('')
  const [newKeyRateLimitTpm, setNewKeyRateLimitTpm] = useState('')
  const [newKeySelectedGroups, setNewKeySelectedGroups] = useState<string[]>([])
  const [newKeyResult, setNewKeyResult] = useState<ApiKey | null>(null)

  // ─── Edit form state ────────────────────────────────────
  const [editName, setEditName] = useState('')
  const [editSupportedModels, setEditSupportedModels] = useState('')
  const [editRateLimitRpm, setEditRateLimitRpm] = useState('')
  const [editRateLimitTpm, setEditRateLimitTpm] = useState('')
  const [editSelectedGroups, setEditSelectedGroups] = useState<string[]>([])

  // ─── Groups ─────────────────────────────────────────────
  const { data: groupsData } = useGroups()
  const availableGroups = groupsData?.items ?? []

  // ─── Mutations ──────────────────────────────────────────
  const createMutation = useCreateApiKey()
  const updateMutation = useUpdateApiKey()
  const deleteMutation = useDeleteApiKey()

  // ─── Helpers ────────────────────────────────────────────
  const formatGroups = (groups: string | null) => {
    if (!groups) return '全部'
    const list = groups.split(',').map((s) => s.trim()).filter(Boolean)
    if (list.length === 0) return '全部'
    if (list.length <= 3) return list.join(', ')
    return `${list.slice(0, 3).join(', ')} 等 ${list.length} 个`
  }

  const copyToClipboard = async (key: string) => {
    await navigator.clipboard.writeText(key)
    setCopiedKey(key)
    setTimeout(() => setCopiedKey((prev) => (prev === key ? null : prev)), 2000)
  }

  const toggleGroup = (
    current: string[],
    setter: (v: string[]) => void,
    group: string,
  ) => {
    setter(
      current.includes(group)
        ? current.filter((g) => g !== group)
        : [...current, group],
    )
  }

  // ─── Create ─────────────────────────────────────────────
  const handleCreate = () => {
    if (!newKeyName.trim()) return
    const data: CreateApiKeyRequest = {
      name: newKeyName.trim(),
      supported_models: newKeySupportedModels || undefined,
      rate_limit_rpm: newKeyRateLimitRpm ? parseInt(newKeyRateLimitRpm) : undefined,
      rate_limit_tpm: newKeyRateLimitTpm ? parseInt(newKeyRateLimitTpm) : undefined,
      allowed_groups: newKeySelectedGroups.length > 0 ? newKeySelectedGroups.join(',') : undefined,
    }
    createMutation.mutate(data, {
      onSuccess: (key) => {
        setNewKeyResult(key)
        toast.success('API Key 创建成功')
      },
      onError: (err: unknown) => {
        toast.error(`创建失败: ${err instanceof Error ? err.message : String(err)}`)
      },
    })
  }

  // ─── Edit ───────────────────────────────────────────────
  const openEdit = (key: ApiKey) => {
    setEditingKey(key)
    setEditName(key.name)
    setEditSupportedModels(key.supported_models || '')
    setEditRateLimitRpm(key.rate_limit_rpm > 0 ? String(key.rate_limit_rpm) : '')
    setEditRateLimitTpm(key.rate_limit_tpm > 0 ? String(key.rate_limit_tpm) : '')
    setEditSelectedGroups(
      key.allowed_groups
        ? key.allowed_groups.split(',').map((s) => s.trim()).filter(Boolean)
        : [],
    )
    setFormOpen(true)
  }

  const handleUpdate = () => {
    if (!editingKey) return
    updateMutation.mutate(
      {
        id: editingKey.id,
        data: {
          name: editName.trim(),
          supported_models: editSupportedModels || undefined,
          rate_limit_rpm: editRateLimitRpm ? parseInt(editRateLimitRpm) : undefined,
          rate_limit_tpm: editRateLimitTpm ? parseInt(editRateLimitTpm) : undefined,
          allowed_groups: editSelectedGroups.length > 0 ? editSelectedGroups.join(',') : undefined,
        },
      },
      {
        onSuccess: () => {
          setEditingKey(null)
          setFormOpen(false)
          table.refresh()
          toast.success('API Key 更新成功')
        },
        onError: (err: unknown) => {
          toast.error(`更新失败: ${err instanceof Error ? err.message : String(err)}`)
        },
      },
    )
  }

  // ─── Toggle enabled ─────────────────────────────────────
  const handleToggleEnabled = (key: ApiKey) => {
    updateMutation.mutate(
      { id: key.id, data: { enabled: !key.enabled } },
      {
        onSuccess: () => {
          table.refresh()
          toast.success(key.enabled ? 'API Key 已禁用' : 'API Key 已启用')
        },
        onError: (err: unknown) => {
          toast.error(`操作失败: ${err instanceof Error ? err.message : String(err)}`)
        },
      },
    )
  }

  // ─── Delete ─────────────────────────────────────────────
  const handleDelete = () => {
    if (!deleteId) return
    deleteMutation.mutate(deleteId, {
      onSuccess: () => {
        setDeleteId(null)
        table.refresh()
        toast.success('API Key 已删除')
      },
      onError: (err: unknown) => {
        toast.error(`删除失败: ${err instanceof Error ? err.message : String(err)}`)
      },
    })
  }

  // ─── Form open/close ────────────────────────────────────
  const openCreate = () => {
    setEditingKey(null)
    setNewKeyResult(null)
    setNewKeyName('')
    setNewKeySupportedModels('')
    setNewKeyRateLimitRpm('')
    setNewKeyRateLimitTpm('')
    setNewKeySelectedGroups([])
    setFormOpen(true)
  }

  const closeForm = () => {
    setFormOpen(false)
    setEditingKey(null)
    setNewKeyResult(null)
    setNewKeyName('')
    setNewKeySupportedModels('')
    setNewKeyRateLimitRpm('')
    setNewKeyRateLimitTpm('')
    setNewKeySelectedGroups([])
  }

  const isFiltered = table.search || table.status

  // ─── Group selector (reusable for create & edit) ───────
  const GroupSelector = ({
    selected,
    onToggle,
  }: {
    selected: string[]
    onToggle: (group: string) => void
  }) => (
    <div>
      <label className="block text-sm font-medium mb-2">
        允许访问的分组（留空表示全部）
      </label>
      {availableGroups.length === 0 ? (
        <p className="text-xs text-muted-foreground">暂无可用分组</p>
      ) : (
        <div className="max-h-48 overflow-y-auto rounded-lg border p-2 space-y-1">
          {availableGroups.map((group) => (
            <label
              key={group.id}
              className="flex items-center gap-2 px-2 py-1.5 rounded hover:bg-muted/50 cursor-pointer"
            >
              <input
                type="checkbox"
                checked={selected.includes(group.name)}
                onChange={() => onToggle(group.name)}
                className="rounded"
              />
              <span className="text-sm">{group.name}</span>
            </label>
          ))}
        </div>
      )}
      {selected.length > 0 && (
        <p className="text-xs text-muted-foreground mt-1">
          已选择 {selected.length} 个分组
        </p>
      )}
    </div>
  )

  return (
    <div className="space-y-4">
      {/* Page Header */}
      <PageHeader
        title="API Key 管理"
        subtitle="管理客户端访问密钥"
        action={
          <Button onClick={openCreate}>
            <Plus className="mr-2 h-4 w-4" />
            创建 API Key
          </Button>
        }
      />

      {/* Filter Bar */}
      <FilterBar
        searchValue={table.searchInput}
        onSearchChange={table.setSearchInput}
        searchPlaceholder="搜索 Key 名称..."
        statusValue={table.status}
        onStatusChange={table.setStatus}
        statusOptions={[
          { label: '启用', value: 'enabled' },
          { label: '禁用', value: 'disabled' },
        ]}
        onRefresh={table.refresh}
        loading={table.loading}
      />

      {/* Table */}
      <div className="rounded-2xl border bg-card overflow-hidden">
        <div className="overflow-x-auto">
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
              <EmptyState
                loading={table.loading}
                isEmpty={!table.loading && displayData.items.length === 0}
                loadingText="加载中..."
                emptyText={isFiltered ? '没有匹配的 API Key' : '暂无 API Key，点击上方按钮创建'}
                colSpan={7}
              />
              {!table.loading &&
                displayData.items.map((apiKey) => (
                  <tr
                    key={apiKey.id}
                    className="border-b last:border-0 hover:bg-muted/30 transition-colors"
                  >
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
                          {copiedKey === apiKey.api_key ? (
                            <Check className="h-3.5 w-3.5" />
                          ) : (
                            <Copy className="h-3.5 w-3.5" />
                          )}
                        </button>
                      </div>
                    </td>
                    <td className="px-4 py-3 text-xs text-muted-foreground">
                      {formatGroups(apiKey.allowed_groups)}
                    </td>
                    <td className="px-4 py-3 text-xs text-muted-foreground">
                      {apiKey.rate_limit_rpm > 0 || apiKey.rate_limit_tpm > 0
                        ? `${apiKey.rate_limit_rpm || '∞'} RPM / ${apiKey.rate_limit_tpm || '∞'} TPM`
                        : '不限'}
                    </td>
                    <td className="px-4 py-3 text-center">
                      <StatusBadge
                        enabled={apiKey.enabled}
                        onClick={() => handleToggleEnabled(apiKey)}
                      />
                    </td>
                    <td className="px-4 py-3 text-muted-foreground text-xs">
                      {formatDate(apiKey.created_at)}
                    </td>
                    <td className="px-4 py-3">
                      <div className="flex items-center justify-center gap-1">
                        <Button
                          variant="ghost"
                          size="icon"
                          className="h-8 w-8"
                          onClick={() => openEdit(apiKey)}
                          title="编辑"
                        >
                          <Pencil className="h-3.5 w-3.5" />
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon"
                          className="h-8 w-8 text-destructive hover:text-destructive"
                          onClick={() => setDeleteId(apiKey.id)}
                          title="删除"
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </Button>
                      </div>
                    </td>
                  </tr>
                ))}
            </tbody>
          </table>
        </div>
      </div>

      {/* Create/Edit Dialog */}
      <Dialog
        open={formOpen}
        onOpenChange={(open) => {
          if (!open) closeForm()
        }}
      >
        <DialogContent className="max-w-md max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>
              {editingKey
                ? '编辑 API Key'
                : newKeyResult
                  ? 'API Key 创建成功'
                  : '创建 API Key'}
            </DialogTitle>
          </DialogHeader>

          {/* Creation success state */}
          {newKeyResult ? (
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
                    onClick={() => copyToClipboard(newKeyResult.api_key)}
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
                <Button variant="outline" onClick={closeForm}>
                  我已保存
                </Button>
              </div>
            </div>
          ) : editingKey ? (
            /* Edit form */
            <div className="space-y-4">
              <div>
                <label className="block text-sm font-medium mb-1">名称</label>
                <input
                  type="text"
                  value={editName}
                  onChange={(e) => setEditName(e.target.value)}
                  className="input"
                  placeholder="例如：前端应用"
                />
              </div>
              <div>
                <label className="block text-sm font-medium mb-1">
                  支持的模型（逗号分隔，留空表示全部）
                </label>
                <input
                  type="text"
                  value={editSupportedModels}
                  onChange={(e) => setEditSupportedModels(e.target.value)}
                  className="input"
                  placeholder="例如：gpt-4, claude-sonnet-4"
                />
              </div>
              <GroupSelector
                selected={editSelectedGroups}
                onToggle={(g) => toggleGroup(editSelectedGroups, setEditSelectedGroups, g)}
              />
              <div>
                <label className="block text-sm font-medium mb-2">速率限制</label>
                <div className="grid grid-cols-2 gap-3">
                  <div>
                    <label className="block text-xs text-muted-foreground mb-1">
                      RPM（请求/分钟）
                    </label>
                    <input
                      type="number"
                      value={editRateLimitRpm}
                      onChange={(e) => setEditRateLimitRpm(e.target.value)}
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
                      value={editRateLimitTpm}
                      onChange={(e) => setEditRateLimitTpm(e.target.value)}
                      className="input"
                      placeholder="不限"
                    />
                  </div>
                </div>
              </div>
              <div className="flex justify-end gap-2">
                <Button variant="outline" onClick={closeForm}>
                  取消
                </Button>
                <Button
                  onClick={handleUpdate}
                  disabled={!editName.trim() || updateMutation.isPending}
                  className="btn-primary"
                >
                  {updateMutation.isPending ? '保存中...' : '保存'}
                </Button>
              </div>
            </div>
          ) : (
            /* Create form */
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
                <label className="block text-sm font-medium mb-1">
                  支持的模型（逗号分隔，留空表示全部）
                </label>
                <input
                  type="text"
                  value={newKeySupportedModels}
                  onChange={(e) => setNewKeySupportedModels(e.target.value)}
                  className="input"
                  placeholder="例如：gpt-4, claude-sonnet-4"
                />
              </div>
              <GroupSelector
                selected={newKeySelectedGroups}
                onToggle={(g) => toggleGroup(newKeySelectedGroups, setNewKeySelectedGroups, g)}
              />
              <div>
                <label className="block text-sm font-medium mb-2">速率限制</label>
                <div className="grid grid-cols-2 gap-3">
                  <div>
                    <label className="block text-xs text-muted-foreground mb-1">
                      RPM（请求/分钟）
                    </label>
                    <input
                      type="number"
                      value={newKeyRateLimitRpm}
                      onChange={(e) => setNewKeyRateLimitRpm(e.target.value)}
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
                      value={newKeyRateLimitTpm}
                      onChange={(e) => setNewKeyRateLimitTpm(e.target.value)}
                      className="input"
                      placeholder="不限"
                    />
                  </div>
                </div>
              </div>
              <div className="flex justify-end gap-2">
                <Button variant="outline" onClick={closeForm}>
                  取消
                </Button>
                <Button
                  onClick={handleCreate}
                  disabled={!newKeyName.trim() || createMutation.isPending}
                  className="btn-primary"
                >
                  {createMutation.isPending ? '创建中...' : '创建'}
                </Button>
              </div>
            </div>
          )}
        </DialogContent>
      </Dialog>

      {/* Delete Confirm Dialog */}
      <ConfirmDeleteDialog
        open={!!deleteId}
        onOpenChange={(open) => {
          if (!open) setDeleteId(null)
        }}
        message="确定要删除此 API Key 吗？使用该 Key 的应用将无法继续访问。"
        onConfirm={handleDelete}
      />
    </div>
  )
}
