import {
  AlertCircle,
  CheckCircle2,
  Download,
  ExternalLink,
  Loader2,
} from 'lucide-react'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import type { UpdateCheck } from '@/api/types'

interface UpdateCheckDialogProps {
  data: UpdateCheck | undefined
  isLoading: boolean
  isError: boolean
  open: boolean
  onOpenChange: (open: boolean) => void
}

export function UpdateCheckDialog({
  data,
  isLoading,
  isError,
  open,
  onOpenChange,
}: UpdateCheckDialogProps) {
  const hasUpdate = data?.has_update ?? false

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            {isLoading ? (
              <>
                <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
                检查更新中
              </>
            ) : isError ? (
              <>
                <AlertCircle className="h-5 w-5 text-red-500" />
                检查失败
              </>
            ) : hasUpdate ? (
              <>
                <AlertCircle className="h-5 w-5 text-primary" />
                发现新版本
              </>
            ) : (
              <>
                <CheckCircle2 className="h-5 w-5 text-emerald-500" />
                已是最新
              </>
            )}
          </DialogTitle>
          <DialogDescription>
            {isLoading
              ? '正在获取最新版本信息...'
              : isError
                ? '无法连接 GitHub，请检查网络或稍后再试'
                : hasUpdate
                  ? `galaxy-router v${data?.latest_version} 已发布`
                  : '当前已是最新版本'}
          </DialogDescription>
        </DialogHeader>

        {!isLoading && !isError && data && (
          <div className="space-y-3">
            <div className="flex items-center justify-between rounded-lg border bg-muted/30 px-3 py-2 text-sm">
              <span className="text-muted-foreground">当前版本</span>
              <code className="font-mono">v{data.current_version}</code>
            </div>

            {hasUpdate && (
              <>
                <div className="flex items-center justify-between rounded-lg border bg-primary/5 px-3 py-2 text-sm">
                  <span className="text-muted-foreground">最新版本</span>
                  <code className="font-mono font-medium text-primary">
                    v{data.latest_version}
                  </code>
                </div>

                {data.release_notes && (
                  <div className="max-h-60 overflow-y-auto rounded-lg border p-3">
                    <p className="mb-1.5 text-xs font-medium text-muted-foreground">更新内容</p>
                    <pre className="whitespace-pre-wrap break-words font-sans text-xs">
                      {data.release_notes}
                    </pre>
                  </div>
                )}

                <Button
                  className="w-full"
                  onClick={() =>
                    window.open(data.release_url, '_blank', 'noopener,noreferrer')
                  }
                >
                  <Download className="mr-2 h-4 w-4" />
                  前往 GitHub 下载
                  <ExternalLink className="ml-2 h-3 w-3" />
                </Button>
              </>
            )}
          </div>
        )}

        {isError && (
          <p className="text-center text-xs text-muted-foreground">
            提示：国内访问 GitHub 可能不稳定，可配置 HTTPS_PROXY 环境变量或稍后再试。
          </p>
        )}
      </DialogContent>
    </Dialog>
  )
}
