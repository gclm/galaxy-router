import { useEffect, useState } from 'react'
import { useLocation, useNavigate } from 'react-router-dom'
import { useAuthStore } from '@/stores/auth'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { User, LogOut, Settings, Sun, Moon, Monitor, PanelLeftClose, PanelLeft, Download } from 'lucide-react'
import { useUpdateCheck } from '@/api/query-hooks'
import { UpdateCheckDialog } from '@/components/UpdateCheckDialog'

type Theme = 'light' | 'dark' | 'system'

function getStoredTheme(): Theme {
  return (localStorage.getItem('theme') as Theme) || 'system'
}

function applyTheme(theme: Theme) {
  const root = document.documentElement
  if (theme === 'dark' || (theme === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches)) {
    root.classList.add('dark')
  } else {
    root.classList.remove('dark')
  }
}

const themeIcons: Record<Theme, typeof Sun> = {
  light: Sun,
  dark: Moon,
  system: Monitor,
}

interface PageInfo {
  title: string
}

const pageMap: [string, PageInfo][] = [
  ['/', { title: '仪表盘' }],
  ['/channels', { title: '渠道管理' }],
  ['/routes', { title: '模型路由管理' }],
  ['/api-keys', { title: 'API Keys' }],
  ['/stats/models', { title: '模型统计' }],
  ['/stats/channels', { title: '渠道统计' }],
  ['/api-key-stats', { title: 'Key 统计' }],
  ['/logs', { title: '请求日志' }],
  ['/playground', { title: '操练场' }],
  ['/models', { title: '模型信息' }],
  ['/settings', { title: '设置' }],
]

function getPageInfo(pathname: string): PageInfo {
  // 精确匹配 / 优先
  if (pathname === '/') return pageMap[0][1]
  // 前缀匹配，长的优先
  const sorted = pageMap.filter(([p]) => p !== '/' && pathname.startsWith(p))
    .sort((a, b) => b[0].length - a[0].length)
  return sorted[0]?.[1] ?? { title: '管理面板' }
}

export function Header({ collapsed, onToggleCollapse }: { collapsed: boolean; onToggleCollapse: () => void }) {
  const { user, logout } = useAuthStore()
  const navigate = useNavigate()
  const location = useLocation()
  const [theme, setTheme] = useState<Theme>(getStoredTheme())
  const [updateOpen, setUpdateOpen] = useState(false)
  const updateCheck = useUpdateCheck()

  useEffect(() => {
    applyTheme(theme)
    localStorage.setItem('theme', theme)
  }, [theme])

  const cycleTheme = () => {
    const order: Theme[] = ['light', 'dark', 'system']
    const next = order[(order.indexOf(theme) + 1) % order.length]
    setTheme(next)
  }

  const ThemeIcon = themeIcons[theme]
  const { title } = getPageInfo(location.pathname)

  return (
    <header className="flex h-16 items-center justify-between border-b px-4">
      <div className="flex items-center gap-3">
        <Button
          variant="ghost"
          size="icon"
          onClick={onToggleCollapse}
          title={collapsed ? '展开侧边栏' : '折叠侧边栏'}
        >
          {collapsed ? <PanelLeft className="h-5 w-5" /> : <PanelLeftClose className="h-5 w-5" />}
        </Button>
        <h2 className="text-lg font-semibold flex items-center gap-2">
          <span className="h-2 w-2 rounded-full bg-primary shadow-sm shadow-primary/50" />
          {title}
        </h2>
      </div>
      <div className="flex items-center gap-1">
        <Button
          variant="ghost"
          size="icon"
          onClick={() => setUpdateOpen(true)}
          title="检查更新"
          className="relative"
        >
          <Download className="h-5 w-5" />
          {updateCheck.data?.has_update && (
            <span className="absolute right-1.5 top-1.5 h-2 w-2 rounded-full bg-red-500 ring-2 ring-background" />
          )}
        </Button>
        <Button
          variant="ghost"
          size="icon"
          onClick={cycleTheme}
          title={theme === 'light' ? '亮色模式' : theme === 'dark' ? '暗色模式' : '跟随系统'}
        >
          <ThemeIcon className="h-5 w-5" />
        </Button>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="icon">
              <User className="h-5 w-5" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem disabled>
              <span className="text-sm text-muted-foreground">
                {user?.username}
              </span>
            </DropdownMenuItem>
            <DropdownMenuItem onClick={() => navigate('/settings')}>
              <Settings className="mr-2 h-4 w-4" />
              <span>设置</span>
            </DropdownMenuItem>
            <DropdownMenuItem onClick={logout}>
              <LogOut className="mr-2 h-4 w-4" />
              <span>退出登录</span>
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
      <UpdateCheckDialog
        data={updateCheck.data}
        isLoading={updateCheck.isLoading}
        isError={updateCheck.isError}
        open={updateOpen}
        onOpenChange={setUpdateOpen}
      />
    </header>
  )
}
