import { useState } from 'react'
import { channelsApi } from '@/api/channels'
import type { Channel, CreateChannelRequest } from '@/api/types'
import { ENDPOINT_LABELS } from '@/api/types'
import { Button } from '@/components/ui/button'
import { StatusBadge } from '@/components/StatusBadge'
import { ConfirmDeleteDialog } from '@/components/ConfirmDeleteDialog'
import { ChannelForm } from '@/components/ChannelForm'
import { ChannelDetail } from '@/components/ChannelDetail'
import { TestModelDialog } from '@/components/TestModelDialog'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { FilterBar, PageHeader, DataTable, SortHeader } from '@/components/common'
import { useTableLoader } from '@/hooks/useTableLoader'
import {
  useCreateChannel,
  useUpdateChannel,
  useDeleteChannel,
} from '@/api/query-hooks'
import { formatDate } from '@/lib/utils'
import { toast } from 'sonner'
import {
  Plus,
  Pencil,
  Trash2,
  FlaskConical,
} from 'lucide-react'

export function Channels() {
  // ─── Table state via useTableLoader（服务端分页） ───────
  const table = useTableLoader<Channel>({
    fetchFn: async (params) => {
      const result = await channelsApi.list({
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
  const [editingChannel, setEditingChannel] = useState<Channel | null>(null)
  const [detailChannel, setDetailChannel] = useState<Channel | null>(null)
  const [testChannel, setTestChannel] = useState<Channel | null>(null)
  const [deleteId, setDeleteId] = useState<string | null>(null)
  const [toggleTarget, setToggleTarget] = useState<Channel | null>(null)

  // ─── Mutations ──────────────────────────────────────────
  const createMutation = useCreateChannel()
  const updateMutation = useUpdateChannel()
  const deleteMutation = useDeleteChannel()

  const handleCreate = async (data: CreateChannelRequest) => {
    createMutation.mutate(data, {
      onSuccess: () => {
        setFormOpen(false)
        table.refresh()
        toast.success('渠道创建成功')
      },
      onError: (err: unknown) => {
        toast.error(`创建失败: ${err instanceof Error ? err.message : String(err)}`)
      },
    })
  }

  const handleUpdate = async (data: CreateChannelRequest) => {
    if (!editingChannel) return
    updateMutation.mutate(
      { id: editingChannel.id, data },
      {
        onSuccess: () => {
          setEditingChannel(null)
          setFormOpen(false)
          table.refresh()
          toast.success('渠道更新成功')
        },
        onError: (err: unknown) => {
          toast.error(`更新失败: ${err instanceof Error ? err.message : String(err)}`)
        },
      },
    )
  }

  const handleToggleEnabled = (channel: Channel) => {
    updateMutation.mutate(
      { id: channel.id, data: { enabled: !channel.enabled } },
      {
        onSuccess: () => {
          table.refresh()
          toast.success(channel.enabled ? '渠道已禁用' : '渠道已启用')
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
        toast.success('渠道已删除')
      },
      onError: (err: unknown) => {
        toast.error(`删除失败: ${err instanceof Error ? err.message : String(err)}`)
      },
    })
  }

  // ─── Helpers ────────────────────────────────────────────
  const openEdit = (channel: Channel) => {
    setEditingChannel(channel)
    setFormOpen(true)
  }

  const openCreate = () => {
    setEditingChannel(null)
    setFormOpen(true)
  }

  const closeForm = () => {
    setFormOpen(false)
    setEditingChannel(null)
  }

  const isFiltered = table.search || table.status

  const columns = [
    { header: <SortHeader label="名称" field="name" sortBy={table.sortBy} sortOrder={table.sortOrder} onSort={table.handleSort} /> },
    { header: '上游端点' },
    { header: '状态', align: 'center' as const },
    { header: '模型', align: 'center' as const },
    { header: 'Keys', align: 'center' as const },
    { header: <SortHeader label="创建时间" field="created_at" sortBy={table.sortBy} sortOrder={table.sortOrder} onSort={table.handleSort} /> },
    { header: '操作', align: 'center' as const },
  ]

  return (
    <div className="space-y-4">
      {/* Page Header */}
      <PageHeader
        title="渠道管理"
        subtitle="管理上游服务渠道与 API Key"
        action={
          <Button onClick={openCreate}>
            <Plus className="mr-2 h-4 w-4" />
            创建渠道
          </Button>
        }
      />

      {/* Filter Bar */}
      <FilterBar
        searchValue={table.searchInput}
        onSearchChange={table.setSearchInput}
        searchPlaceholder="搜索渠道名称..."
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
        emptyText={isFiltered ? '没有匹配的渠道' : '暂无渠道，点击上方按钮添加'}
        pagination={{
          total: table.total,
          page: table.page,
          pageSize: table.pageSize,
          onPageChange: table.setPage,
          onPageSizeChange: table.setPageSize,
          pageSizeOptions: [20, 50, 100],
        }}
      >
        {table.data.map((channel) => {
          const models = channel.models || []
          return (
            <tr
              key={channel.id}
              className="border-b last:border-0 hover:bg-muted/30 transition-colors"
            >
              <td className="px-4 py-3 font-medium">
                <button
                  className="hover:text-primary hover:underline cursor-pointer text-left"
                  onClick={() => setDetailChannel(channel)}
                >
                  {channel.name}
                </button>
              </td>
              <td className="px-4 py-3">
                <div className="flex flex-wrap gap-1">
                  {channel.endpoints.map((ep, i) => (
                    <span
                      key={i}
                      className={`inline-flex items-center rounded-md px-1.5 py-0.5 text-xs font-medium ${
                        ep.enabled === false
                          ? 'bg-muted text-muted-foreground line-through'
                          : 'bg-primary/10 text-primary'
                      }`}
                    >
                      {ENDPOINT_LABELS[ep.type] || ep.type}
                    </span>
                  ))}
                </div>
              </td>
              <td className="px-4 py-3 text-center">
                <StatusBadge
                  enabled={channel.enabled}
                  onClick={() => setToggleTarget(channel)}
                />
              </td>
              <td className="px-4 py-3 text-center text-muted-foreground">
                {models.length}
              </td>
              <td className="px-4 py-3 text-center text-muted-foreground">
                {channel.api_keys.filter((k) => k.enabled !== false).length}/
                {channel.api_keys.length}
              </td>
              <td className="px-4 py-3 text-muted-foreground text-xs">
                {formatDate(channel.created_at)}
              </td>
              <td className="px-4 py-3">
                <div className="flex items-center justify-center gap-1">
                  <Button
                    variant="ghost"
                    size="icon"
                    className="h-8 w-8"
                    onClick={() => setTestChannel(channel)}
                    title="测试"
                  >
                    <FlaskConical className="h-3.5 w-3.5" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="h-8 w-8"
                    onClick={() => openEdit(channel)}
                    title="编辑"
                  >
                    <Pencil className="h-3.5 w-3.5" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="h-8 w-8 text-destructive hover:text-destructive"
                    onClick={() => setDeleteId(channel.id)}
                    title="删除"
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </Button>
                </div>
              </td>
            </tr>
          )
        })}
      </DataTable>

      {/* Create/Edit Dialog */}
      <Dialog
        open={formOpen}
        onOpenChange={(open) => {
          if (!open) closeForm()
        }}
      >
        <DialogContent className="max-w-2xl max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>
              {editingChannel ? '编辑渠道' : '创建渠道'}
            </DialogTitle>
          </DialogHeader>
          <ChannelForm
            channel={editingChannel ?? undefined}
            onSubmit={editingChannel ? handleUpdate : handleCreate}
            onCancel={closeForm}
            onTest={(ch) => setTestChannel(ch)}
          />
        </DialogContent>
      </Dialog>

      {/* Test Dialog */}
      <TestModelDialog
        channel={testChannel}
        open={!!testChannel}
        onOpenChange={(open) => {
          if (!open) setTestChannel(null)
        }}
      />

      {/* Detail Dialog */}
      <ChannelDetail
        channel={detailChannel}
        open={!!detailChannel}
        onOpenChange={(open) => {
          if (!open) setDetailChannel(null)
        }}
        onEdit={() => {
          if (detailChannel) {
            openEdit(detailChannel)
            setDetailChannel(null)
          }
        }}
        onTest={() => {
          if (detailChannel) {
            setTestChannel(detailChannel)
            setDetailChannel(null)
          }
        }}
      />

      {/* Delete Confirm Dialog */}
      <ConfirmDeleteDialog
        open={!!deleteId}
        onOpenChange={(open) => {
          if (!open) setDeleteId(null)
        }}
        message="确定要删除此渠道吗？此操作不可撤销。"
        onConfirm={handleDelete}
      />

      {/* Toggle Confirm Dialog */}
      <ConfirmDeleteDialog
        open={!!toggleTarget}
        onOpenChange={(open) => {
          if (!open) setToggleTarget(null)
        }}
        title={toggleTarget?.enabled ? '禁用渠道' : '启用渠道'}
        message={`确定要${toggleTarget?.enabled ? '禁用' : '启用'}渠道「${toggleTarget?.name}」吗？`}
        onConfirm={handleToggleConfirm}
      />
    </div>
  )
}
