import { useMemo, useState } from 'react'
import type { Channel, Group, CreateGroupRequest } from '@/api/types'
import { Button } from '@/components/ui/button'
import { Check, Plus, Search, Sparkles, Trash2, X } from 'lucide-react'

interface GroupFormProps {
  group?: Group
  channels: Channel[]
  onSubmit: (data: CreateGroupRequest) => Promise<void>
  onCancel: () => void
}

interface SelectedItem {
  channel_id: string
  model_name: string
  priority: number
  weight: number
}

function memberKey(channelId: string, modelName: string) {
  return `${channelId}::${modelName}`
}

export function GroupForm({ group, channels, onSubmit, onCancel }: GroupFormProps) {
  const [name, setName] = useState(group?.name ?? '')
  const [provider, setProvider] = useState(group?.provider ?? '')
  const [matchRegex, setMatchRegex] = useState(group?.match_regex ?? '')
  // 重试配置已废弃（调度器改为遍历全部候选），UI 不再暴露；retry_enabled 仍提交
  const retryEnabled = group?.retry_enabled ?? true
  const [firstTokenTimeout, setFirstTokenTimeout] = useState(group?.first_token_timeout_secs?.toString() ?? '30')
  const [enabled, setEnabled] = useState(group?.enabled ?? true)
  const [submitting, setSubmitting] = useState(false)
  const [modelSearch, setModelSearch] = useState('')

  const [selectedItems, setSelectedItems] = useState<SelectedItem[]>(
    group?.items.map(item => ({
      channel_id: item.channel_id,
      model_name: item.model_name,
      priority: item.priority,
      weight: item.weight,
    })) ?? []
  )

  const channelMap = new Map(channels.map(ch => [ch.id, ch]))
  const selectedKeys = useMemo(() => new Set(selectedItems.map(i => memberKey(i.channel_id, i.model_name))), [selectedItems])

  // 按渠道分组的模型列表（带搜索过滤）
  const channelGroups = useMemo(() => {
    const keyword = modelSearch.trim().toLowerCase()
    return channels
      .filter(ch => ch.models.length > 0)
      .map(ch => {
        const models = keyword
          ? ch.models.filter(m => m.toLowerCase().includes(keyword))
          : ch.models
        const selectedCount = ch.models.filter(m => selectedKeys.has(memberKey(ch.id, m))).length
        return { ...ch, filteredModels: models, selectedCount }
      })
      .filter(ch => ch.filteredModels.length > 0 || (keyword && ch.name.toLowerCase().includes(keyword)))
  }, [channels, modelSearch, selectedKeys])

  // 自动添加：按分组名称匹配模型
  const handleAutoAdd = () => {
    const keyword = matchRegex || name
    if (!keyword) return
    const isRegex = !!matchRegex
    let re: RegExp | null = null
    if (isRegex) {
      try { re = new RegExp(matchRegex) } catch { return }
    }
    const toAdd: SelectedItem[] = []
    for (const ch of channels) {
      for (const model of ch.models) {
        const key = memberKey(ch.id, model)
        if (selectedKeys.has(key)) continue
        const matches = isRegex ? re!.test(model) : model === keyword
        if (matches) toAdd.push({ channel_id: ch.id, model_name: model, priority: 1, weight: 100 })
      }
    }
    if (toAdd.length > 0) setSelectedItems(prev => [...prev, ...toAdd])
  }

  const autoAddDisabled = useMemo(() => {
    const keyword = matchRegex || name
    if (!keyword) return true
    let re: RegExp | null = null
    if (matchRegex) { try { re = new RegExp(matchRegex) } catch { return true } }
    return channels.every(ch =>
      ch.models.every(m => {
        const matches = matchRegex ? re!.test(m) : m === keyword
        return !matches || selectedKeys.has(memberKey(ch.id, m))
      })
    )
  }, [channels, matchRegex, name, selectedKeys])

  const handleAddModel = (channelId: string, modelName: string) => {
    const key = memberKey(channelId, modelName)
    if (selectedKeys.has(key)) return
    setSelectedItems(prev => [...prev, { channel_id: channelId, model_name: modelName, priority: 1, weight: 100 }])
  }

  const handleRemoveItem = (index: number) => {
    setSelectedItems(prev => prev.filter((_, i) => i !== index))
  }

  const handleClearItems = () => setSelectedItems([])

  const handlePriorityChange = (index: number, value: number) => {
    setSelectedItems(prev => prev.map((item, i) => i === index ? { ...item, priority: value } : item))
  }

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!name || selectedItems.length === 0) return
    setSubmitting(true)
    try {
      const data: CreateGroupRequest = {
        name,
        provider: provider || undefined,
        match_regex: matchRegex || undefined,
        retry_enabled: retryEnabled,
        first_token_timeout_secs: parseInt(firstTokenTimeout) || 30,
        enabled,
        items: selectedItems,
      }
      await onSubmit(data)
    } finally {
      setSubmitting(false)
    }
  }

  const isValid = name.trim() && selectedItems.length > 0

  return (
    <form onSubmit={handleSubmit} className="flex flex-col h-full min-h-0 px-1">
      {/* 顶部：基本信息 */}
      <div className="space-y-3 mb-4">
        <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
          <div>
            <label className="block text-sm font-medium mb-1">分组名称 *</label>
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              className="input"
              placeholder="例如：gpt-4o"
              required
            />
          </div>
          <div>
            <label className="block text-sm font-medium mb-1">厂家 (Provider)</label>
            <input
              type="text"
              value={provider}
              onChange={(e) => setProvider(e.target.value)}
              className="input"
              placeholder="留空自动识别（如 openai）"
            />
            <p className="text-xs text-muted-foreground mt-1">根据分组名自动匹配模型信息中的厂家</p>
          </div>
          <div>
            <label className="block text-sm font-medium mb-1">匹配规则 (正则)</label>
            <input
              type="text"
              value={matchRegex}
              onChange={(e) => setMatchRegex(e.target.value)}
              className="input font-mono"
              placeholder="留空则精确匹配分组名"
            />
          </div>
        </div>
        <div className="flex items-center gap-4">
          <label className="flex items-center gap-2 text-sm cursor-pointer">
            <input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} className="rounded" />
            启用
          </label>
          {/* 重试配置已移除：调度器改为遍历全部候选（外层 for candidate），
              max_retries/retry_enabled 字段保留在 DB 但代码不读，详见 task 2026-07-01-智能调度打分重构 */}
          <label className="flex items-center gap-1.5 text-sm">
            <span className="text-muted-foreground">首字超时</span>
            <input
              type="number"
              value={firstTokenTimeout}
              onChange={(e) => setFirstTokenTimeout(e.target.value)}
              className="input w-16 text-center h-7 text-xs"
              min="0"
            />
            <span className="text-muted-foreground">秒</span>
          </label>
        </div>
      </div>

      {/* 底部：双面板 */}
      <div className="flex-1 min-h-0 grid grid-cols-1 md:grid-cols-2 gap-3">
        {/* 左侧：模型选择器 */}
        <div className="rounded-xl border bg-muted/30 flex flex-col min-h-0">
          <div className="flex items-center justify-between px-3 py-2 border-b bg-muted/50">
            <span className="text-sm font-medium">从渠道添加模型</span>
            <div className="flex items-center gap-2">
              <div className="relative">
                <Search className="absolute left-2 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground pointer-events-none" />
                <input
                  value={modelSearch}
                  onChange={(e) => setModelSearch(e.target.value)}
                  className="h-7 rounded-lg border-border/60 bg-background/70 pl-7 pr-2 text-xs w-32 focus-visible:border-border/60 focus-visible:ring-0"
                  placeholder="搜索模型..."
                />
              </div>
              <button
                type="button"
                onClick={handleAutoAdd}
                disabled={autoAddDisabled}
                className={`flex items-center gap-1 px-2 py-1 rounded-lg text-xs font-medium transition-colors ${
                  autoAddDisabled ? 'text-muted-foreground/50 cursor-not-allowed' : 'hover:bg-muted text-muted-foreground hover:text-foreground'
                }`}
                title={!name.trim() ? '请先填写分组名称' : '按分组名称自动匹配添加'}
              >
                <Sparkles className="h-3.5 w-3.5" />
                <span>自动添加</span>
              </button>
            </div>
          </div>
          <div className="flex-1 min-h-0 overflow-y-auto p-2 space-y-1">
            {channels.length === 0 ? (
              <div className="text-center py-8 text-sm text-muted-foreground">暂无渠道，请先在渠道管理中创建</div>
            ) : channelGroups.length === 0 ? (
              <div className="text-center py-8 text-sm text-muted-foreground">所有渠道均未配置模型<br />请先在渠道管理中为渠道添加模型</div>
            ) : (
              channelGroups.map(ch => (
                <div key={ch.id} className="rounded-lg border border-border/40 overflow-hidden">
                  <div className="flex items-center gap-3 px-3 py-2 text-sm bg-muted/60">
                    <span className="flex-1 truncate font-medium">{ch.name}</span>
                    <span className="text-xs text-muted-foreground">{ch.selectedCount}/{ch.models.length}</span>
                  </div>
                  <div className="p-1.5 space-y-1">
                    {ch.filteredModels.map(model => {
                      const isSelected = selectedKeys.has(memberKey(ch.id, model))
                      return (
                        <button
                          key={model}
                          type="button"
                          onClick={() => !isSelected && handleAddModel(ch.id, model)}
                          disabled={isSelected}
                          className={`w-full flex items-center justify-between gap-2 rounded-lg border px-2.5 py-2 text-left text-sm transition-colors ${
                            isSelected ? 'opacity-50 cursor-not-allowed border-border/30 bg-muted/30' : 'border-border/50 bg-background hover:bg-muted'
                          }`}
                        >
                          <span className="min-w-0 truncate">{model}</span>
                          {isSelected ? (
                            <Check className="h-4 w-4 text-primary shrink-0" />
                          ) : (
                            <Plus className="h-4 w-4 text-muted-foreground shrink-0" />
                          )}
                        </button>
                      )
                    })}
                  </div>
                </div>
              ))
            )}
          </div>
        </div>

        {/* 右侧：已选成员 */}
        <div className="rounded-xl border bg-muted/30 flex flex-col min-h-0">
          <div className="flex items-center justify-between px-3 py-2 border-b bg-muted/50">
            <span className="text-sm font-medium">
              已选成员
              {selectedItems.length > 0 && (
                <span className="ml-1.5 text-xs text-muted-foreground font-normal">({selectedItems.length})</span>
              )}
            </span>
            <button
              type="button"
              onClick={handleClearItems}
              disabled={selectedItems.length === 0}
              className={`flex items-center gap-1 px-2 py-1 rounded-lg text-xs font-medium transition-colors ${
                selectedItems.length === 0 ? 'text-muted-foreground/50 cursor-not-allowed' : 'hover:bg-muted text-muted-foreground hover:text-foreground'
              }`}
              title="清空所有"
            >
              <Trash2 className="h-3.5 w-3.5" />
              <span>清空</span>
            </button>
          </div>
          <div className="flex-1 min-h-0 overflow-y-auto">
            {selectedItems.length === 0 ? (
              <div className="text-center py-8 text-sm text-muted-foreground">从左侧选择模型添加</div>
            ) : (
              <div className="p-2 space-y-1.5">
                {selectedItems.map((item, index) => {
                  const channelName = channelMap.get(item.channel_id)?.name || item.channel_id
                  return (
                    <div key={memberKey(item.channel_id, item.model_name)} className="flex items-center gap-2 rounded-lg border border-border/50 bg-background px-3 py-2">
                      <span className="text-xs text-muted-foreground w-5 shrink-0">{index + 1}</span>
                      <div className="flex-1 min-w-0">
                        <div className="text-sm font-medium truncate">{item.model_name}</div>
                        <div className="text-xs text-muted-foreground">{channelName}</div>
                      </div>
                      <input
                        type="number"
                        value={item.priority}
                        onChange={(e) => handlePriorityChange(index, parseInt(e.target.value) || 1)}
                        className="input w-14 text-center text-xs h-7"
                        min="1"
                        title="优先级（数值小=优先选用，同档内按速度/并发打分）"
                      />
                      <button
                        type="button"
                        onClick={() => handleRemoveItem(index)}
                        className="shrink-0 text-muted-foreground hover:text-destructive transition-colors"
                      >
                        <X className="h-4 w-4" />
                      </button>
                    </div>
                  )
                })}
              </div>
            )}
          </div>
        </div>
      </div>

      {/* 操作按钮 */}
      <div className="flex justify-end gap-2 pt-3 mt-3 border-t">
        <Button type="button" variant="outline" onClick={onCancel}>取消</Button>
        <Button type="submit" disabled={!isValid || submitting} className="btn-primary">
          {submitting ? '保存中...' : group ? '更新' : '创建'}
        </Button>
      </div>
    </form>
  )
}
