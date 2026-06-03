import { useEffect, useRef, useState } from 'react'
import { channelsApi, type ChannelListParams } from '@/api/channels'
import type { Channel, CreateChannelRequest } from '@/api/types'
import { ENDPOINT_LABELS } from '@/api/types'
import { Button } from '@/components/ui/button'
import { Pagination } from '@/components/Pagination'
import { StatusBadge } from '@/components/StatusBadge'
import { ConfirmDeleteDialog } from '@/components/ConfirmDeleteDialog'
import { useDebouncedValue } from '@/lib/hooks'
import { ChannelForm } from '@/components/ChannelForm'
import { ChannelDetail } from '@/components/ChannelDetail'
import { TestModelDialog } from '@/components/TestModelDialog'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { formatDate } from '@/lib/utils'
import {
  Plus,
  Pencil,
  Trash2,
  FlaskConical,
  Search,
  RefreshCw,
  ArrowUpDown,
} from 'lucide-react'

export function Channels() {
  const [channels, setChannels] = useState<Channel[]>([])
  const [total, setTotal] = useState(0)
  const [loading, setLoading] = useState(true)

  // 筛选状态
  const [searchInput, setSearchInput] = useState('')
  const search = useDebouncedValue(searchInput)
  const [status, setStatus] = useState<string>('')
  const [sortBy, setSortBy] = useState('created_at')
  const [sortOrder, setSortOrder] = useState<'asc' | 'desc'>('desc')
  const [page, setPage] = useState(1)
  const [pageSize, setPageSize] = useState(20)

  // Dialog 状态
  const [formOpen, setFormOpen] = useState(false)
  const [editingChannel, setEditingChannel] = useState<Channel | null>(null)
  const [detailChannel, setDetailChannel] = useState<Channel | null>(null)
  const [testChannel, setTestChannel] = useState<Channel | null>(null)

  const fetchRef = useRef<() => void>(() => {})

  // 删除确认
  const [deleteId, setDeleteId] = useState<string | null>(null)

  // 搜索变化时重置页码
  const prevSearch = useRef(search)
  useEffect(() => {
    if (search !== prevSearch.current) {
      prevSearch.current = search
      setPage(1)
    }
  }, [search])

  useEffect(() => {
    const controller = new AbortController()
    const run = async () => {
      setLoading(true)
      try {
        const params: ChannelListParams = {
          search: search || undefined,
          status: status || undefined,
          sort_by: sortBy,
          sort_order: sortOrder,
          page,
          page_size: pageSize,
        }
        const data = await channelsApi.list(params)
        if (!controller.signal.aborted) {
          setChannels(data.items)
          setTotal(data.total)
        }
      } catch (error) {
        if (!controller.signal.aborted) console.error('Failed to fetch channels:', error)
      } finally {
        if (!controller.signal.aborted) setLoading(false)
      }
    }
    run()
    fetchRef.current = run
    return () => { controller.abort() }
  }, [search, status, sortBy, sortOrder, page, pageSize])

  const handleCreate = async (data: CreateChannelRequest) => {
    await channelsApi.create(data)
    setFormOpen(false)
    fetchRef.current()
  }

  const handleUpdate = async (data: CreateChannelRequest) => {
    if (!editingChannel) return
    await channelsApi.update(editingChannel.id, data)
    setEditingChannel(null)
    setFormOpen(false)
    fetchRef.current()
  }

  const handleToggleEnabled = async (channel: Channel) => {
    await channelsApi.update(channel.id, { enabled: !channel.enabled })
    fetchRef.current()
  }

  const handleDelete = async () => {
    if (!deleteId) return
    await channelsApi.delete(deleteId)
    setDeleteId(null)
    fetchRef.current()
  }

  const handleSort = (field: string) => {
    if (sortBy === field) {
      setSortOrder(sortOrder === 'asc' ? 'desc' : 'asc')
    } else {
      setSortBy(field)
      setSortOrder('desc')
    }
    setPage(1)
  }

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


  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <p className="text-sm text-muted-foreground">管理上游服务渠道与 API Key</p>
        </div>
        <Button onClick={openCreate} className="btn-primary">
          <Plus className="mr-2 h-4 w-4" />
          添加渠道
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
            placeholder="搜索渠道名称..."
            className="input pl-9"
          />
        </div>
        <select
          value={status}
          onChange={(e) => { setStatus(e.target.value); setPage(1) }}
          className="input w-28"
        >
          <option value="">全部状态</option>
          <option value="enabled">启用</option>
          <option value="disabled">禁用</option>
        </select>
        <Button variant="outline" size="icon" onClick={() => fetchRef.current()} title="刷新">
          <RefreshCw className="h-4 w-4" />
        </Button>
      </div>

      {/* 表格 */}
      <div className="rounded-2xl border bg-card overflow-hidden">
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b bg-muted/50">
                <th className="text-left px-4 py-3 font-medium">
                  <button className="inline-flex items-center gap-1 hover:text-foreground" onClick={() => handleSort('name')}>
                    名称
                    {sortBy === 'name' && <ArrowUpDown className="h-3 w-3" />}
                  </button>
                </th>
                <th className="text-left px-4 py-3 font-medium">上游端点</th>
                <th className="text-center px-4 py-3 font-medium">状态</th>
                <th className="text-center px-4 py-3 font-medium">模型</th>
                <th className="text-center px-4 py-3 font-medium">Keys</th>
                <th className="text-left px-4 py-3 font-medium">
                  <button className="inline-flex items-center gap-1 hover:text-foreground" onClick={() => handleSort('created_at')}>
                    创建时间
                    {sortBy === 'created_at' && <ArrowUpDown className="h-3 w-3" />}
                  </button>
                </th>
                <th className="text-center px-4 py-3 font-medium">操作</th>
              </tr>
            </thead>
            <tbody>
              {loading ? (
                <tr>
                  <td colSpan={7} className="text-center py-12 text-muted-foreground">加载中...</td>
                </tr>
              ) : channels.length === 0 ? (
                <tr>
                  <td colSpan={7} className="text-center py-12 text-muted-foreground">
                    {search || status ? '没有匹配的渠道' : '暂无渠道，点击上方按钮添加'}
                  </td>
                </tr>
              ) : (
                channels.map((channel) => {
                  const models = channel.models || []
                  return (
                    <tr key={channel.id} className="border-b last:border-0 hover:bg-muted/30 transition-colors">
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
                            <span key={i} className={`inline-flex items-center rounded-md px-1.5 py-0.5 text-xs font-medium ${ep.enabled === false ? 'bg-muted text-muted-foreground line-through' : 'bg-primary/10 text-primary'}`}>
                              {ENDPOINT_LABELS[ep.type] || ep.type}
                            </span>
                          ))}
                        </div>
                      </td>
                      <td className="px-4 py-3 text-center">
                        <StatusBadge enabled={channel.enabled} onClick={() => handleToggleEnabled(channel)} />
                      </td>
                      <td className="px-4 py-3 text-center text-muted-foreground">{models.length}</td>
                      <td className="px-4 py-3 text-center text-muted-foreground">
                        {channel.api_keys.filter(k => k.enabled !== false).length}/{channel.api_keys.length}
                      </td>
                      <td className="px-4 py-3 text-muted-foreground text-xs">{formatDate(channel.created_at)}</td>
                      <td className="px-4 py-3">
                        <div className="flex items-center justify-center gap-1">
                          <Button variant="ghost" size="icon" className="h-8 w-8" onClick={() => setTestChannel(channel)} title="测试">
                            <FlaskConical className="h-3.5 w-3.5" />
                          </Button>
                          <Button variant="ghost" size="icon" className="h-8 w-8" onClick={() => openEdit(channel)} title="编辑">
                            <Pencil className="h-3.5 w-3.5" />
                          </Button>
                          <Button variant="ghost" size="icon" className="h-8 w-8 text-destructive hover:text-destructive" onClick={() => setDeleteId(channel.id)} title="删除">
                            <Trash2 className="h-3.5 w-3.5" />
                          </Button>
                        </div>
                      </td>
                    </tr>
                  )
                })
              )}
            </tbody>
          </table>
        </div>

        <Pagination total={total} page={page} pageSize={pageSize} onPageChange={setPage} onPageSizeChange={setPageSize} pageSizeOptions={[20, 50, 100]} />
      </div>

      {/* 创建/编辑 Dialog */}
      <Dialog open={formOpen} onOpenChange={(open) => { if (!open) closeForm() }}>
        <DialogContent className="max-w-2xl max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>{editingChannel ? '编辑渠道' : '创建渠道'}</DialogTitle>
          </DialogHeader>
          <ChannelForm
            channel={editingChannel ?? undefined}
            onSubmit={editingChannel ? handleUpdate : handleCreate}
            onCancel={closeForm}
          />
        </DialogContent>
      </Dialog>

      {/* 测试 Dialog */}
      <TestModelDialog
        channel={testChannel}
        open={!!testChannel}
        onOpenChange={(open) => { if (!open) setTestChannel(null) }}
      />

      {/* 详情 Dialog */}
      <ChannelDetail
        channel={detailChannel}
        open={!!detailChannel}
        onOpenChange={(open) => { if (!open) setDetailChannel(null) }}
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

      {/* 删除确认 Dialog */}
      <ConfirmDeleteDialog
        open={!!deleteId}
        onOpenChange={(open) => { if (!open) setDeleteId(null) }}
        message="确定要删除此渠道吗？此操作不可撤销。"
        onConfirm={handleDelete}
      />
    </div>
  )
}
