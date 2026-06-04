import { useEffect, useState, useMemo } from 'react'
import { groupsApi } from '@/api/groups'
import { channelsApi } from '@/api/channels'
import type { Group, Channel, CreateGroupRequest } from '@/api/types'
import { Button } from '@/components/ui/button'
import { Pagination } from '@/components/Pagination'
import { StatusBadge } from '@/components/StatusBadge'
import { ConfirmDeleteDialog } from '@/components/ConfirmDeleteDialog'
import { GroupForm } from '@/components/GroupForm'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { FilterBar } from '@/components/common/FilterBar'
import { PageHeader } from '@/components/common/PageHeader'
import { EmptyState } from '@/components/common/EmptyState'
import { useTableLoader } from '@/hooks/useTableLoader'
import {
  useCreateGroup,
  useUpdateGroup,
  useDeleteGroup,
} from '@/api/query-hooks'
import { formatDate } from '@/lib/utils'
import { toast } from 'sonner'
import {
  Plus,
  Pencil,
  Trash2,
  ArrowUpDown,
} from 'lucide-react'

export function Groups() {
  // ─── Channels for GroupForm ─────────────────────────────
  const [channels, setChannels] = useState<Channel[]>([])
  useEffect(() => {
    channelsApi.list().then(res => setChannels(res.items)).catch(console.error)
  }, [])

  // ─── Table state via useTableLoader ─────────────────────
  const table = useTableLoader<Group>({
    fetchFn: async () => {
      const result = await groupsApi.list()
      return { items: result.items, total: result.total }
    },
    defaultPageSize: 20,
  })

  // ─── Client-side filter + sort + paginate ───────────────
  const displayData = useMemo(() => {
    let items = [...table.data]

    // Filter by search
    if (table.search) {
      const q = table.search.toLowerCase()
      items = items.filter((g) => g.name.toLowerCase().includes(q))
    }

    // Filter by status
    if (table.status === 'enabled') {
      items = items.filter((g) => g.enabled)
    } else if (table.status === 'disabled') {
      items = items.filter((g) => !g.enabled)
    }

    // Sort
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

    // Paginate
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
  const [editingGroup, setEditingGroup] = useState<Group | null>(null)
  const [deleteId, setDeleteId] = useState<string | null>(null)

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
      <div className="rounded-2xl border bg-card overflow-hidden">
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b bg-muted/50">
                <th className="text-left px-4 py-3 font-medium">
                  <button
                    className="inline-flex items-center gap-1 hover:text-foreground"
                    onClick={() => table.handleSort('name')}
                  >
                    名称
                    {table.sortBy === 'name' && <ArrowUpDown className="h-3 w-3" />}
                  </button>
                </th>
                <th className="text-left px-4 py-3 font-medium">匹配规则</th>
                <th className="text-center px-4 py-3 font-medium">渠道数</th>
                <th className="text-center px-4 py-3 font-medium">重试</th>
                <th className="text-center px-4 py-3 font-medium">状态</th>
                <th className="text-left px-4 py-3 font-medium">
                  <button
                    className="inline-flex items-center gap-1 hover:text-foreground"
                    onClick={() => table.handleSort('created_at')}
                  >
                    创建时间
                    {table.sortBy === 'created_at' && <ArrowUpDown className="h-3 w-3" />}
                  </button>
                </th>
                <th className="text-center px-4 py-3 font-medium">操作</th>
              </tr>
            </thead>
            <tbody>
              <EmptyState
                loading={table.loading}
                isEmpty={!table.loading && displayData.items.length === 0}
                loadingText="加载中..."
                emptyText={isFiltered ? '没有匹配的分组' : '暂无分组，点击上方按钮添加'}
                colSpan={7}
              />
              {!table.loading &&
                displayData.items.map((group) => (
                  <tr
                    key={group.id}
                    className="border-b last:border-0 hover:bg-muted/30 transition-colors"
                  >
                    <td className="px-4 py-3 font-medium">{group.name}</td>
                    <td className="px-4 py-3">
                      {group.match_regex ? (
                        <code className="rounded bg-muted px-1.5 py-0.5 text-xs">{group.match_regex}</code>
                      ) : (
                        <span className="text-muted-foreground text-xs">精确匹配</span>
                      )}
                    </td>
                    <td className="px-4 py-3 text-center text-muted-foreground">{group.items.length}</td>
                    <td className="px-4 py-3 text-center text-muted-foreground text-xs">
                      {group.retry_enabled ? `${group.max_retries} 次` : '关闭'}
                    </td>
                    <td className="px-4 py-3 text-center">
                      <StatusBadge
                        enabled={group.enabled}
                        onClick={() => handleToggleEnabled(group)}
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
            </tbody>
          </table>
        </div>

        <Pagination
          total={displayData.total}
          page={table.page}
          pageSize={table.pageSize}
          onPageChange={table.setPage}
          onPageSizeChange={table.setPageSize}
          pageSizeOptions={[20, 50, 100]}
        />
      </div>

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
    </div>
  )
}
