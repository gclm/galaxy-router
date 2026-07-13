import { apiClient } from './client'
import type {
  Route,
  RouteItem,
  CreateRouteRequest,
  UpdateRouteRequest,
  AddRouteItemRequest,
  PaginatedResponse,
} from './types'

export interface RouteListParams {
  search?: string
  status?: string
  sort_by?: string
  sort_order?: string
  page?: number
  page_size?: number
}

export const routesApi = {
  list: (params?: RouteListParams) =>
    apiClient.get<PaginatedResponse<Route>>('/routes', params as Record<string, string | number | undefined>),

  get: (id: string) => apiClient.get<Route>(`/routes/${id}`),

  create: (data: CreateRouteRequest) =>
    apiClient.post<Route>('/routes', data),

  update: (id: string, data: UpdateRouteRequest) =>
    apiClient.put<Route>(`/routes/${id}`, data),

  delete: (id: string) => apiClient.delete<void>(`/routes/${id}`),

  addItem: (groupId: string, data: AddRouteItemRequest) =>
    apiClient.post<RouteItem>(`/routes/${groupId}/items`, data),

  deleteItem: (groupId: string, itemId: string) =>
    apiClient.delete<void>(`/routes/${groupId}/items/${itemId}`),
}
