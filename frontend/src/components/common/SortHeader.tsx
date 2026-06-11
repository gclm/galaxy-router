import { ArrowUp, ArrowDown } from 'lucide-react'

interface SortHeaderProps {
  label: string
  field: string
  sortBy: string
  sortOrder: 'asc' | 'desc'
  onSort: (field: string) => void
}

export function SortHeader({ label, field, sortBy, sortOrder, onSort }: SortHeaderProps) {
  const isActive = sortBy === field
  return (
    <button
      type="button"
      className="inline-flex items-center gap-1 hover:text-foreground"
      onClick={() => onSort(field)}
    >
      {label}
      {isActive && (
        sortOrder === 'asc'
          ? <ArrowUp className="h-3 w-3" />
          : <ArrowDown className="h-3 w-3" />
      )}
    </button>
  )
}
