import { apiClient } from './client'
import type { ApiKey, CreateApiKeyRequest, UpdateApiKeyRequest, PaginatedResponse } from './types'

export interface ApiKeyListParams {
  search?: string
  status?: string
  sort_by?: string
  sort_order?: string
  page?: number
  page_size?: number
}

export const apiKeysApi = {
  list: (params?: ApiKeyListParams) =>
    apiClient.get<PaginatedResponse<ApiKey>>('/api-keys', params as Record<string, string | number | undefined>),

  get: (id: string) => apiClient.get<ApiKey>(`/api-keys/${id}`),

  create: (data: CreateApiKeyRequest) =>
    apiClient.post<ApiKey>('/api-keys', data),

  update: (id: string, data: UpdateApiKeyRequest) =>
    apiClient.put<ApiKey>(`/api-keys/${id}`, data),

  delete: (id: string) => apiClient.delete<void>(`/api-keys/${id}`),
}
