import type { ReactNode } from 'react'

interface PageHeaderProps {
  title?: string
  subtitle?: string
  action?: ReactNode
}

export function PageHeader({ subtitle, action }: PageHeaderProps) {
  if (!subtitle && !action) return null

  return (
    <div className="flex items-center justify-between gap-4">
      <div>
        {subtitle && <p className="text-xs text-muted-foreground">{subtitle}</p>}
      </div>
      {action && <div className="flex items-center gap-2">{action}</div>}
    </div>
  )
}
