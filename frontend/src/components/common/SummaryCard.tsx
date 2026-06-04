interface SummaryCardProps {
  label: string
  value: string
}

export function SummaryCard({ label, value }: SummaryCardProps) {
  return (
    <div className="rounded-xl bg-muted/30 border border-border p-4">
      <p className="text-xs font-medium text-muted-foreground">{label}</p>
      <p className="text-lg font-bold tracking-tight mt-1">{value}</p>
    </div>
  )
}
