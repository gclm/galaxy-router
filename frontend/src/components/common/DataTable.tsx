import type { ReactNode } from 'react'
import { Pagination } from '@/components/Pagination'
import { EmptyState } from './EmptyState'

interface Column {
  header: ReactNode
  align?: 'left' | 'center' | 'right'
  className?: string
}

interface DataTablePagination {
  total: number
  page: number
  pageSize: number
  onPageChange: (page: number) => void
  onPageSizeChange?: (size: number) => void
  pageSizeOptions?: number[]
}

interface DataTableProps {
  columns: Column[]
  loading?: boolean
  isEmpty?: boolean
  emptyText?: string
  loadingText?: string
  pagination?: DataTablePagination
  children: ReactNode
}

const alignMap: Record<string, string> = {
  left: 'text-left',
  center: 'text-center',
  right: 'text-right',
}

export function DataTable({
  columns,
  loading = false,
  isEmpty = false,
  emptyText = '暂无数据',
  loadingText = '加载中...',
  pagination,
  children,
}: DataTableProps) {
  const showEmpty = loading || isEmpty

  return (
    <div className="rounded-2xl border bg-card overflow-hidden">
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b bg-muted/50">
              {columns.map((col, i) => (
                <th
                  key={i}
                  className={`${alignMap[col.align ?? 'left']} px-4 py-3 font-medium ${col.className ?? ''}`}
                >
                  {col.header}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {showEmpty ? (
              <EmptyState
                loading={loading}
                isEmpty={isEmpty}
                loadingText={loadingText}
                emptyText={emptyText}
                colSpan={columns.length}
              />
            ) : children}
          </tbody>
        </table>
      </div>
      {pagination && (
        <Pagination
          total={pagination.total}
          page={pagination.page}
          pageSize={pagination.pageSize}
          onPageChange={pagination.onPageChange}
          onPageSizeChange={pagination.onPageSizeChange}
          pageSizeOptions={pagination.pageSizeOptions}
        />
      )}
    </div>
  )
}
