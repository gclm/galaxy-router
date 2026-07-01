import { useState } from 'react'
import { groupsApi } from '@/api/groups'
import type { Group, CreateGroupRequest } from '@/api/types'
import { Button } from '@/components/ui/button'
import { StatusBadge } from '@/components/StatusBadge'
import { ConfirmDeleteDialog } from '@/components/ConfirmDeleteDialog'
import { GroupForm } from '@/components/GroupForm'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { FilterBar, PageHeader, DataTable, SortHeader } from '@/components/common'
import { useTableLoader } from '@/hooks/useTableLoader'
import {
  useCreateGroup,
  useUpdateGroup,
  useDeleteGroup,
  useChannels,
} from '@/api/query-hooks'
import { formatDate } from '@/lib/utils'
import { toast } from 'sonner'
import {
  Plus,
  Pencil,
  Trash2,
} from 'lucide-react'

export function Groups() {
  // ─── Channels for GroupForm（React Query） ──────────────
  const { data: channelsData } = useChannels()
  const channels = channelsData?.items ?? []

  // ─── Table state via useTableLoader（服务端分页） ───────
  const table = useTableLoader<Group>({
    fetchFn: async (params) => {
      const result = await groupsApi.list({
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
  const [editingGroup, setEditingGroup] = useState<Group | null>(null)
  const [deleteId, setDeleteId] = useState<string | null>(null)
  const [toggleTarget, setToggleTarget] = useState<Group | null>(null)

  // ─── Mutations ──────────────────────────────────────────
  const createMutation = useCreateGroup()
  const updateMutation = useUpdateGroup()
  const deleteMutation = useDeleteGroup()

  const handleCreate = async (data: CreateGroupRequest) => {
    createMutation.mutate(data, {
      onSuccess: () => {
        setFormOpen(false)
        table.refresh()
        toast.success('分组创建成功')
      },
      onError: (err: unknown) => {
        toast.error(`创建失败: ${err instanceof Error ? err.message : String(err)}`)
      },
    })
  }

  const handleUpdate = async (data: CreateGroupRequest) => {
    if (!editingGroup) return
    updateMutation.mutate(
      { id: editingGroup.id, data },
      {
        onSuccess: () => {
          setEditingGroup(null)
          setFormOpen(false)
          table.refresh()
          toast.success('分组更新成功')
        },
        onError: (err: unknown) => {
          toast.error(`更新失败: ${err instanceof Error ? err.message : String(err)}`)
        },
      },
    )
  }

  const handleToggleEnabled = (group: Group) => {
    updateMutation.mutate(
      { id: group.id, data: { enabled: !group.enabled } },
      {
        onSuccess: () => {
          table.refresh()
          toast.success(group.enabled ? '分组已禁用' : '分组已启用')
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

  const handleDelete = () => {
    if (!deleteId) return
    deleteMutation.mutate(deleteId, {
      onSuccess: () => {
        setDeleteId(null)
        table.refresh()
        toast.success('分组已删除')
      },
      onError: (err: unknown) => {
        toast.error(`删除失败: ${err instanceof Error ? err.message : String(err)}`)
      },
    })
  }

  // ─── Helpers ────────────────────────────────────────────
  const openEdit = (group: Group) => {
    setEditingGroup(group)
    setFormOpen(true)
  }

  const openCreate = () => {
    setEditingGroup(null)
    setFormOpen(true)
  }

  const closeForm = () => {
    setFormOpen(false)
    setEditingGroup(null)
  }

  const isFiltered = table.search || table.status

  const columns = [
    { header: <SortHeader label="名称" field="name" sortBy={table.sortBy} sortOrder={table.sortOrder} onSort={table.handleSort} /> },
    { header: '厂家' },
    { header: '匹配规则' },
    { header: '渠道数', align: 'center' as const },
    { header: '重试', align: 'center' as const },
    { header: '状态', align: 'center' as const },
    { header: <SortHeader label="创建时间" field="created_at" sortBy={table.sortBy} sortOrder={table.sortOrder} onSort={table.handleSort} /> },
    { header: '操作', align: 'center' as const },
  ]

  return (
    <div className="space-y-4">
      {/* Page Header */}
      <PageHeader
        title="分组管理"
        subtitle="配置模型分组与负载均衡策略"
        action={
          <Button onClick={openCreate}>
            <Plus className="mr-2 h-4 w-4" />
            添加分组
          </Button>
        }
      />

      {/* Filter Bar */}
      <FilterBar
        searchValue={table.searchInput}
        onSearchChange={table.setSearchInput}
        searchPlaceholder="搜索分组名称..."
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
        emptyText={isFiltered ? '没有匹配的分组' : '暂无分组，点击上方按钮添加'}
        pagination={{
          total: table.total,
          page: table.page,
          pageSize: table.pageSize,
          onPageChange: table.setPage,
          onPageSizeChange: table.setPageSize,
          pageSizeOptions: [20, 50, 100],
        }}
      >
        {table.data.map((group) => (
          <tr
            key={group.id}
            className="border-b last:border-0 hover:bg-muted/30 transition-colors"
          >
            <td className="px-4 py-3 font-medium">{group.name}</td>
            <td className="px-4 py-3 text-xs text-muted-foreground">
              {group.provider || ''}
            </td>
            <td className="px-4 py-3">
              {group.match_regex ? (
                <code className="rounded bg-muted px-1.5 py-0.5 text-xs">{group.match_regex}</code>
              ) : (
                <span className="text-muted-foreground text-xs">精确匹配</span>
              )}
            </td>
            <td className="px-4 py-3 text-center text-muted-foreground">{group.items.length}</td>
            <td className="px-4 py-3 text-center text-muted-foreground text-xs">
              自动
            </td>
            <td className="px-4 py-3 text-center">
              <StatusBadge
                enabled={group.enabled}
                onClick={() => setToggleTarget(group)}
              />
            </td>
            <td className="px-4 py-3 text-muted-foreground text-xs">
              {formatDate(group.created_at)}
            </td>
            <td className="px-4 py-3">
              <div className="flex items-center justify-center gap-1">
                <Button
                  variant="ghost"
                  size="icon"
                  className="h-8 w-8"
                  onClick={() => openEdit(group)}
                  title="编辑"
                >
                  <Pencil className="h-3.5 w-3.5" />
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  className="h-8 w-8 text-destructive hover:text-destructive"
                  onClick={() => setDeleteId(group.id)}
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
        <DialogContent className="max-w-4xl max-h-[90vh] overflow-hidden flex flex-col">
          <DialogHeader>
            <DialogTitle>{editingGroup ? '编辑分组' : '创建分组'}</DialogTitle>
          </DialogHeader>
          <GroupForm
            group={editingGroup ?? undefined}
            channels={channels}
            onSubmit={editingGroup ? handleUpdate : handleCreate}
            onCancel={closeForm}
          />
        </DialogContent>
      </Dialog>

      {/* Delete Confirm Dialog */}
      <ConfirmDeleteDialog
        open={!!deleteId}
        onOpenChange={(open) => {
          if (!open) setDeleteId(null)
        }}
        message="确定要删除此分组吗？此操作不可撤销。"
        onConfirm={handleDelete}
      />

      {/* Toggle Confirm Dialog */}
      <ConfirmDeleteDialog
        open={!!toggleTarget}
        onOpenChange={(open) => {
          if (!open) setToggleTarget(null)
        }}
        title={toggleTarget?.enabled ? '禁用分组' : '启用分组'}
        message={`确定要${toggleTarget?.enabled ? '禁用' : '启用'}分组「${toggleTarget?.name}」吗？`}
        onConfirm={handleToggleConfirm}
      />
    </div>
  )
}
