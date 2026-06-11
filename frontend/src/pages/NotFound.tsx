import { Button } from '@/components/ui/button'
import { Home, ArrowLeft } from 'lucide-react'

export function NotFound() {
  return (
    <div className="flex items-center justify-center min-h-[60vh]">
      <div className="text-center space-y-4">
        <div className="space-y-2">
          <h1 className="text-6xl font-bold text-muted-foreground/30">404</h1>
          <p className="text-lg font-medium">页面未找到</p>
          <p className="text-sm text-muted-foreground">你访问的页面不存在或已被移除</p>
        </div>
        <div className="flex items-center justify-center gap-3">
          <Button variant="outline" onClick={() => window.history.back()}>
            <ArrowLeft className="mr-2 h-4 w-4" />
            返回上页
          </Button>
          <Button onClick={() => window.location.href = '/'}>
            <Home className="mr-2 h-4 w-4" />
            回到首页
          </Button>
        </div>
      </div>
    </div>
  )
}
