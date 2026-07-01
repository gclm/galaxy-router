import type { Channel } from '@/api/types'
import { ENDPOINT_LABELS } from '@/api/types'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { StatusBadge } from '@/components/StatusBadge'
import { formatDate } from '@/lib/utils'
import { Pencil } from 'lucide-react'

function maskKey(key: string) {
  if (key.length <= 8) return '****'
  return '...' + key.slice(-4)
}

interface ChannelDetailProps {
  channel: Channel | null
  open: boolean
  onOpenChange: (open: boolean) => void
  onEdit: () => void
}

export function ChannelDetail({ channel, open, onOpenChange, onEdit }: ChannelDetailProps) {
  if (!channel) return null

  const enabledKeys = channel.api_keys.filter((k) => k.enabled !== false).length

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>渠道详情</DialogTitle>
        </DialogHeader>

        <div className="space-y-5">
          {/* 基本信息 */}
          <section>
            <h3 className="text-sm font-semibold text-muted-foreground mb-2">基本信息</h3>
            <div className="grid grid-cols-2 gap-3 text-sm">
              <div>
                <span className="text-muted-foreground">名称</span>
                <p className="font-medium">{channel.name}</p>
              </div>
              <div>
                <span className="text-muted-foreground">状态</span>
                <p><StatusBadge enabled={channel.enabled} onClick={() => {}} /></p>
              </div>
              <div>
                <span className="text-muted-foreground">创建时间</span>
                <p className="font-mono text-xs">{formatDate(channel.created_at)}</p>
              </div>
              <div>
                <span className="text-muted-foreground">更新时间</span>
                <p className="font-mono text-xs">{formatDate(channel.updated_at)}</p>
              </div>
            </div>
          </section>

          {/* API Keys */}
          <section>
            <h3 className="text-sm font-semibold text-muted-foreground mb-2">
              API Keys ({enabledKeys}/{channel.api_keys.length} 启用)
            </h3>
            <div className="space-y-1.5">
              {channel.api_keys.map((k, i) => (
                <div
                  key={`${k.key}-${i}`}
                  className="flex items-center gap-3 rounded-lg border px-3 py-2 text-sm"
                >
                  <span className="text-muted-foreground w-4 text-center">{i + 1}</span>
                  <span className="font-medium flex-1 truncate">
                    {k.note?.trim() || maskKey(k.key)}
                  </span>
                  <span className="font-mono text-xs text-muted-foreground">
                    {maskKey(k.key)}
                  </span>
                  <StatusBadge enabled={k.enabled !== false} onClick={() => {}} />
                </div>
              ))}
            </div>
          </section>

          {/* 端点 */}
          <section>
            <h3 className="text-sm font-semibold text-muted-foreground mb-2">
              上游端点 ({channel.endpoints.length})
            </h3>
            <div className="space-y-1.5">
              {channel.endpoints.map((ep, i) => (
                <div
                  key={i}
                  className="flex items-center gap-3 rounded-lg border px-3 py-2 text-sm"
                >
                  <span className={`inline-flex items-center rounded-md px-1.5 py-0.5 text-xs font-medium ${
                    ep.enabled === false ? 'bg-muted text-muted-foreground line-through' : 'bg-primary/10 text-primary'
                  }`}>
                    {ENDPOINT_LABELS[ep.type] || ep.type}
                  </span>
                  <span className="font-mono text-xs text-muted-foreground flex-1 truncate">
                    {ep.base_url}
                  </span>
                </div>
              ))}
            </div>
          </section>

          {/* 模型 */}
          <section>
            <h3 className="text-sm font-semibold text-muted-foreground mb-2">
              模型 ({channel.models.length})
            </h3>
            <div className="flex flex-wrap gap-1.5">
              {channel.models.map((m) => (
                <span key={m} className="inline-flex items-center rounded-md bg-secondary px-2 py-1 text-xs font-medium">
                  {m}
                </span>
              ))}
              {channel.models.length === 0 && (
                <span className="text-muted-foreground text-xs">暂无模型</span>
              )}
            </div>
          </section>

          {/* 高级配置 */}
          <section>
            <h3 className="text-sm font-semibold text-muted-foreground mb-2">高级配置</h3>
            <div className="grid grid-cols-2 sm:grid-cols-3 gap-3 text-sm">
              <div>
                <span className="text-muted-foreground">并发</span>
                <p className="font-medium">{channel.concurrency}</p>
              </div>
              <div>
                <span className="text-muted-foreground">失败阈值</span>
                <p className="font-medium">{channel.failure_threshold}</p>
              </div>
              <div>
                <span className="text-muted-foreground">黑名单时长</span>
                <p className="font-medium">{channel.blacklist_minutes} 分钟</p>
              </div>
              <div>
                <span className="text-muted-foreground">RPM 限制</span>
                <p className="font-medium">{channel.rate_limit_rpm ?? '无限制'}</p>
              </div>
              <div>
                <span className="text-muted-foreground">TPM 限制</span>
                <p className="font-medium">{channel.rate_limit_tpm ?? '无限制'}</p>
              </div>
              <div>
                <span className="text-muted-foreground">超时</span>
                <p className="font-medium">{channel.timeout_secs ?? 300} 秒</p>
              </div>
              <div>
                <span className="text-muted-foreground">最大并发</span>
                <p className="font-medium">{channel.max_concurrency ? channel.max_concurrency : '不限'}</p>
              </div>
            </div>
          </section>
        </div>

        {/* 底部操作 */}
        <div className="flex justify-end gap-2 pt-4 border-t">
          <Button onClick={onEdit}>
            <Pencil className="mr-2 h-4 w-4" />
            编辑
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}
