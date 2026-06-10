import { apiClient } from './client'
import type { StatsOverview, DailyStats, ModelStats, ChannelStats, RequestLog, RequestLogDetail, LatencyStats, BudgetLimit } from './types'

export interface StatsParams {
  days?: number
  start_date?: string
  end_date?: string
}

export interface LogsParams {
  page?: number
  page_size?: number
  model?: string
  channel_id?: string
  status?: string
  api_key_id?: string
}

function statsQuery(params?: StatsParams): Record<string, string | number | undefined> | undefined {
  if (!params) return undefined
  if (params.start_date && params.end_date) {
    return { start_date: params.start_date, end_date: params.end_date }
  }
  if (params.days) return { days: params.days }
  return undefined
}

export const statsApi = {
  overview: () => apiClient.get<StatsOverview>('/stats/overview'),
  daily: (params?: StatsParams) => apiClient.get<DailyStats[]>('/stats/daily', statsQuery(params)),
  models: (params?: StatsParams) => apiClient.get<ModelStats[]>('/stats/models', statsQuery(params)),
  channels: (params?: StatsParams) => apiClient.get<ChannelStats[]>('/stats/channels', statsQuery(params)),
  latency: (params?: StatsParams) => apiClient.get<LatencyStats>('/stats/latency', statsQuery(params)),
  listBudgets: () => apiClient.get<BudgetLimit[]>('/budgets'),
  setBudget: (data: { api_key_id: string; monthly_limit_usd?: number; daily_limit_usd?: number; enabled?: boolean }) =>
    apiClient.post<BudgetLimit>('/budgets', data),
  deleteBudget: (id: string) => apiClient.delete(`/budgets/${id}`),
  logModels: () => apiClient.get<string[]>('/stats/logs/models'),
  logs: (params?: LogsParams) => apiClient.get<{ items: RequestLog[]; total: number }>('/stats/logs', params as Record<string, string | number | undefined>),
  logDetail: (id: string) => apiClient.get<RequestLogDetail>(`/stats/logs/${id}`),
}
