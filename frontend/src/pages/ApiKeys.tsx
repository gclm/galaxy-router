import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import type { ApiKey, CreateApiKeyRequest } from '@/api/types'
import { apiKeysApi } from '@/api/api-keys'
import { Button } from '@/components/ui/button'
import { StatusBadge } from '@/components/StatusBadge'
import { ConfirmDeleteDialog } from '@/components/ConfirmDeleteDialog'
import { ApiKeyForm } from '@/components/ApiKeyForm'
import { FilterBar, PageHeader, DataTable, SortHeader } from '@/components/common'
import { useTableLoader } from '@/hooks/useTableLoader'
import {
  useCreateApiKey,
  useUpdateApiKey,
  useDeleteApiKey,
  useGroups,
  useBudgets,
  useSetBudget,
  useDeleteBudget,
} from '@/api/query-hooks'
import type { BudgetLimit } from '@/api/types'
import { formatDate, maskKey, copyText } from '@/lib/utils'
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
  const navigate = useNavigate()
  // ─── Table state via useTableLoader（服务端分页） ───────
  const table = useTableLoader<ApiKey>({
    fetchFn: async (params) => {
      const result = await apiKeysApi.list({
        page: params.page,
        page_size: params.pageSize,
        search: params.search || undefined,
        status: params.status || undefined,
        sort_by: params.sortBy,
        sort_order: params.sortOrder,
      })
      return result
    },
    defaultPageSize: 20,
  })

  // ─── Dialog state ───────────────────────────────────────
  const [formOpen, setFormOpen] = useState(false)
  const [editingKey, setEditingKey] = useState<ApiKey | null>(null)
  const [deleteId, setDeleteId] = useState<string | null>(null)
  const [toggleTarget, setToggleTarget] = useState<ApiKey | null>(null)
  const [copiedKey, setCopiedKey] = useState<string | null>(null)
  const [newKeyResult, setNewKeyResult] = useState<ApiKey | null>(null)

  // ─── Groups ─────────────────────────────────────────────
  const { data: groupsData } = useGroups()
  const availableGroups = groupsData?.items ?? []

  // ─── Budgets ────────────────────────────────────────────
  const { data: budgets } = useBudgets()
  const setBudgetMutation = useSetBudget()
  const deleteBudgetMutation = useDeleteBudget()

  const getBudgetForKey = (keyId: string): BudgetLimit | undefined =>
    budgets?.find((b) => b.api_key_id === keyId)

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
    await copyText(key)
    setCopiedKey(key)
    setTimeout(() => setCopiedKey((prev) => (prev === key ? null : prev)), 2000)
  }

  // ─── Create ─────────────────────────────────────────────
  const handleCreate = (data: CreateApiKeyRequest & { budget_monthly?: number; budget_daily?: number }) => {
    const { budget_monthly, budget_daily, ...keyData } = data
    createMutation.mutate(keyData, {
      onSuccess: (key) => {
        // 创建时仅当填了预算(>0)才设置,避免给新 key 写无意义的 0 记录
        if ((budget_monthly ?? 0) > 0 || (budget_daily ?? 0) > 0) {
          setBudgetMutation.mutate({
            api_key_id: key.id,
            monthly_limit_usd: budget_monthly ?? 0,
            daily_limit_usd: budget_daily ?? 0,
          })
        }
        setNewKeyResult(key)
        toast.success('API Key 创建成功')
      },
      onError: (err: unknown) => {
        toast.error(`创建失败: ${err instanceof Error ? err.message : String(err)}`)
      },
    })
  }

  // ─── Update ─────────────────────────────────────────────
  const handleUpdate = (data: CreateApiKeyRequest & { budget_monthly?: number; budget_daily?: number }) => {
    if (!editingKey) return
    const { budget_monthly, budget_daily, ...keyData } = data
    updateMutation.mutate(
      { id: editingKey.id, data: keyData },
      {
        onSuccess: () => {
          // 保存预算设置
          if (budget_monthly !== undefined || budget_daily !== undefined) {
            const monthly = budget_monthly ?? 0
            const daily = budget_daily ?? 0
            if (monthly > 0 || daily > 0) {
              setBudgetMutation.mutate({
                api_key_id: editingKey.id,
                monthly_limit_usd: monthly,
                daily_limit_usd: daily,
              })
            } else {
              const existingBudget = getBudgetForKey(editingKey.id)
              if (existingBudget) {
                deleteBudgetMutation.mutate(existingBudget.id)
              }
            }
          }
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

  const handleToggleConfirm = () => {
    if (!toggleTarget) return
    handleToggleEnabled(toggleTarget)
    setToggleTarget(null)
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
    setFormOpen(true)
  }

  const openEdit = (key: ApiKey) => {
    setEditingKey(key)
    setNewKeyResult(null)
    setFormOpen(true)
  }

  const closeForm = () => {
    setFormOpen(false)
    setEditingKey(null)
    setNewKeyResult(null)
  }

  const isFiltered = table.search || table.status

  const isSubmitting = createMutation.isPending || updateMutation.isPending

  const columns = [
    { header: <SortHeader label="名称" field="name" sortBy={table.sortBy} sortOrder={table.sortOrder} onSort={table.handleSort} /> },
    { header: 'Key' },
    { header: '分组权限' },
    { header: '速率限制' },
    { header: '预算' },
    { header: '状态', align: 'center' as const },
    { header: <SortHeader label="创建时间" field="created_at" sortBy={table.sortBy} sortOrder={table.sortOrder} onSort={table.handleSort} /> },
    { header: '操作', align: 'center' as const },
  ]

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
      <DataTable
        columns={columns}
        loading={table.loading}
        isEmpty={!table.loading && table.data.length === 0}
        emptyText={isFiltered ? '没有匹配的 API Key' : '暂无 API Key，点击上方按钮创建'}
        pagination={{
          total: table.total,
          page: table.page,
          pageSize: table.pageSize,
          onPageChange: table.setPage,
          onPageSizeChange: table.setPageSize,
          pageSizeOptions: [20, 50, 100],
        }}
      >
        {table.data.map((apiKey) => (
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
            <td className="px-4 py-3 text-xs text-muted-foreground">
              {(() => {
                const budget = getBudgetForKey(apiKey.id)
                if (!budget || (!budget.monthly_limit_usd && !budget.daily_limit_usd)) {
                  return <span className="text-muted-foreground/50">—</span>
                }
                const parts: string[] = []
                if (budget.monthly_limit_usd > 0) parts.push(`月 $${budget.monthly_limit_usd}`)
                if (budget.daily_limit_usd > 0) parts.push(`日 $${budget.daily_limit_usd}`)
                return <span>{parts.join(' / ')}</span>
              })()}
            </td>
            <td className="px-4 py-3 text-center">
              <StatusBadge
                enabled={apiKey.enabled}
                onClick={() => setToggleTarget(apiKey)}
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
      </DataTable>

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
          <ApiKeyForm
            apiKey={editingKey ?? undefined}
            budget={editingKey ? getBudgetForKey(editingKey.id) : undefined}
            groups={availableGroups}
            onSubmit={editingKey ? handleUpdate : handleCreate}
            onCancel={closeForm}
            newKeyResult={!editingKey ? newKeyResult : undefined}
            onCopy={copyToClipboard}
            copiedKey={copiedKey}
            submitting={isSubmitting}
            onGenerateConfig={(key) => navigate('/client-config', { state: { apiKey: key.api_key } })}
          />
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

      {/* Toggle Confirm Dialog */}
      <ConfirmDeleteDialog
        open={!!toggleTarget}
        onOpenChange={(open) => {
          if (!open) setToggleTarget(null)
        }}
        title={toggleTarget?.enabled ? '禁用 API Key' : '启用 API Key'}
        message={`确定要${toggleTarget?.enabled ? '禁用' : '启用'} API Key「${toggleTarget?.name}」吗？`}
        onConfirm={handleToggleConfirm}
      />
    </div>
  )
}
