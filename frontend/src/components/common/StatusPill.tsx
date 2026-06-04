type StatusVariant = 'success' | 'warning' | 'error' | 'default'

const variantStyles: Record<StatusVariant, string> = {
  success: 'bg-green-500/10 text-green-600 dark:text-green-400 border-green-500/20',
  warning: 'bg-amber-500/10 text-amber-600 dark:text-amber-400 border-amber-500/20',
  error: 'bg-red-500/10 text-red-600 dark:text-red-400 border-red-500/20',
  default: 'bg-muted text-muted-foreground border-border',
}

interface StatusPillProps {
  variant?: StatusVariant
  children: React.ReactNode
  className?: string
}

export function StatusPill({ variant = 'default', children, className = '' }: StatusPillProps) {
  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-xs font-medium border ${variantStyles[variant]} ${className}`}
    >
      {children}
    </span>
  )
}
