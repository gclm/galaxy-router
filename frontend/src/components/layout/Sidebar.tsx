import { Link, useLocation } from 'react-router-dom'
import {
  LayoutDashboard,
  Radio,
  Layers,
  Key,
  BarChart3,
  ScrollText,
  FlaskConical,
  Box,
  Settings,
} from 'lucide-react'
import { cn } from '@/lib/utils'

const navItems = [
  {
    title: '仪表盘',
    href: '/',
    icon: LayoutDashboard,
  },
  {
    title: '渠道管理',
    href: '/channels',
    icon: Radio,
  },
  {
    title: '分组管理',
    href: '/groups',
    icon: Layers,
  },
  {
    title: 'API Keys',
    href: '/api-keys',
    icon: Key,
  },
  {
    title: 'Key 统计',
    href: '/api-key-stats',
    icon: BarChart3,
  },
  {
    title: '请求日志',
    href: '/logs',
    icon: ScrollText,
  },
  {
    title: '操练场',
    href: '/playground',
    icon: FlaskConical,
  },
  {
    title: '模型信息',
    href: '/models',
    icon: Box,
  },
]

export function Sidebar({ collapsed }: { collapsed: boolean }) {
  const location = useLocation()

  return (
    <aside className={`border-r border-sidebar-border bg-sidebar-background flex flex-col transition-all duration-200 ${collapsed ? 'w-16' : 'w-60'}`}>
      <div className={`flex h-16 items-center border-b border-sidebar-border ${collapsed ? 'justify-center px-2' : 'gap-3 px-5'}`}>
        <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-gradient-to-br from-primary to-primary/70 text-primary-foreground text-xs font-bold shadow-sm">
          GR
        </div>
        {!collapsed && (
          <span className="text-sm font-bold text-sidebar-foreground whitespace-nowrap">Galaxy Router</span>
        )}
      </div>
      <nav className="flex-1 space-y-1 p-3">
        {navItems.map((item) => {
          const isActive =
            item.href === '/'
              ? location.pathname === '/'
              : location.pathname.startsWith(item.href)

          return (
            <Link
              key={item.href}
              to={item.href}
              title={collapsed ? item.title : undefined}
              className={cn(
                'group flex items-center gap-3 rounded-xl text-sm font-medium transition-all duration-200',
                collapsed ? 'justify-center px-2 py-2.5' : 'px-3 py-2.5',
                isActive
                  ? 'bg-sidebar-accent text-sidebar-primary border-l-2 border-primary'
                  : 'text-sidebar-foreground/70 hover:bg-sidebar-accent/50 hover:text-sidebar-foreground'
              )}
            >
              <item.icon className={cn('h-4 w-4 shrink-0', isActive && 'text-sidebar-primary')} />
              {!collapsed && <span className="flex-1">{item.title}</span>}
              {isActive && !collapsed && (
                <span className="h-1.5 w-1.5 rounded-full bg-sidebar-primary shadow-sm shadow-sidebar-primary/50" />
              )}
            </Link>
          )
        })}
      </nav>
      <div className="border-t border-sidebar-border p-3">
        <Link
          to="/settings"
          title={collapsed ? '设置' : undefined}
          className={cn(
            'flex items-center gap-3 rounded-xl text-sm font-medium transition-all duration-200',
            collapsed ? 'justify-center px-2 py-2.5' : 'px-3 py-2.5',
            location.pathname === '/settings'
              ? 'bg-sidebar-accent text-sidebar-primary border-l-2 border-primary'
              : 'text-sidebar-foreground/70 hover:bg-sidebar-accent/50 hover:text-sidebar-foreground'
          )}
        >
          <Settings className={cn('h-4 w-4 shrink-0', location.pathname === '/settings' && 'text-sidebar-primary')} />
          {!collapsed && <span>设置</span>}
          {location.pathname === '/settings' && !collapsed && (
            <span className="ml-auto h-1.5 w-1.5 rounded-full bg-sidebar-primary shadow-sm shadow-sidebar-primary/50" />
          )}
        </Link>
      </div>
    </aside>
  )
}
