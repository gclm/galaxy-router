import type { LucideIcon } from 'lucide-react'

interface StatCardProps {
  label: string
  value: React.ReactNode
  subtitle?: React.ReactNode
  icon: LucideIcon
  gradient: string
}

export function StatCard({ label, value, subtitle, icon: Icon, gradient }: StatCardProps) {
  return (
    <div className="rounded-xl bg-muted/30 border border-border p-4 space-y-2 card-hover">
      <div className="flex items-center gap-2.5">
        <div className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-gradient-to-br ${gradient} text-white shadow-sm`}>
          <Icon className="h-4 w-4" />
        </div>
        <span className="text-xs font-medium text-muted-foreground">{label}</span>
      </div>
      <p className="text-xl font-bold tracking-tight">{value}</p>
      {subtitle && (
        <p className="text-[11px] text-muted-foreground/70 leading-tight">{subtitle}</p>
      )}
    </div>
  )
}
