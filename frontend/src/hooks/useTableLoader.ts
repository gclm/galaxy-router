import { useState, useEffect, useCallback, useRef } from 'react'
import { useDebouncedValue } from '@/lib/hooks'

interface UseTableLoaderOptions<TItem, TExtra extends Record<string, unknown> = Record<string, unknown>> {
  /** 数据获取函数，返回 { items, total } 或数组 */
  fetchFn: (params: {
    page: number
    pageSize: number
    search: string
    status: string
    sortBy: string
    sortOrder: 'asc' | 'desc'
    extra: TExtra
    signal?: AbortSignal
  }) => Promise<{ items: TItem[]; total: number } | TItem[]>
  /** 默认每页条数 */
  defaultPageSize?: number
  /** 默认排序字段 */
  defaultSortBy?: string
  /** 默认排序方向 */
  defaultSortOrder?: 'asc' | 'desc'
  /** 搜索防抖延迟(ms) */
  searchDelay?: number
  /** 额外参数 */
  defaultExtra?: TExtra
}

export function useTableLoader<TItem, TExtra extends Record<string, unknown> = Record<string, unknown>>(
  options: UseTableLoaderOptions<TItem, TExtra>,
) {
  const {
    defaultPageSize = 20,
    defaultSortBy = 'created_at',
    defaultSortOrder = 'desc',
    searchDelay = 300,
    defaultExtra = {} as TExtra,
  } = options

  // 用 ref 存储 fetchFn，避免因引用变化触发无限循环；在 effect 中同步最新值（render 中写 ref 违反规则）
  const fetchFnRef = useRef(options.fetchFn)
  useEffect(() => {
    fetchFnRef.current = options.fetchFn
  })

  const [data, setData] = useState<TItem[]>([])
  const [total, setTotal] = useState(0)
  const [loading, setLoading] = useState(true)
  const [page, setPage] = useState(1)
  const [pageSize, setPageSize] = useState(defaultPageSize)
  const [searchInput, setSearchInput] = useState('')
  const [status, setStatus] = useState('')
  const [sortBy, setSortBy] = useState(defaultSortBy)
  const [sortOrder, setSortOrder] = useState<'asc' | 'desc'>(defaultSortOrder)
  const [extra, setExtra] = useState<TExtra>(defaultExtra)
  const [version, setVersion] = useState(0)

  const search = useDebouncedValue(searchInput, searchDelay)
  const abortRef = useRef<AbortController | null>(null)

  const refresh = useCallback(() => {
    setVersion(v => v + 1)
  }, [])

  const handleSort = useCallback((field: string) => {
    setSortOrder(prev => field === sortBy && prev === 'asc' ? 'desc' : 'asc')
    setSortBy(field)
    setPage(1)
  }, [sortBy])

  useEffect(() => {
    const controller = new AbortController()
    abortRef.current = controller

    // eslint-disable-next-line react-hooks/set-state-in-effect -- 数据加载 effect 的 loading 标记（fetch 开始）
    setLoading(true)
    fetchFnRef.current({
      page,
      pageSize,
      search,
      status,
      sortBy,
      sortOrder,
      extra,
      signal: controller.signal,
    })
      .then(result => {
        if (controller.signal.aborted) return
        if (Array.isArray(result)) {
          setData(result)
          setTotal(result.length)
        } else {
          setData(result.items)
          setTotal(result.total)
        }
        setLoading(false)
      })
      .catch(err => {
        if (controller.signal.aborted) return
        console.error('Failed to fetch data:', err)
        setLoading(false)
      })

    return () => controller.abort()
  }, [page, pageSize, search, status, sortBy, sortOrder, extra, version])

  // 搜索/状态变化时重置页码（依赖变重置 state；公共 hook 不改外部 API）
  // eslint-disable-next-line react-hooks/set-state-in-effect
  useEffect(() => { setPage(1) }, [search])
  // eslint-disable-next-line react-hooks/set-state-in-effect
  useEffect(() => { setPage(1) }, [status])

  return {
    data,
    total,
    loading,
    page,
    pageSize,
    search,
    searchInput,
    setSearchInput,
    status,
    setStatus,
    sortBy,
    setSortBy,
    sortOrder,
    setSortOrder,
    extra,
    setExtra,
    setPage,
    setPageSize,
    refresh,
    handleSort,
  } as const
}
