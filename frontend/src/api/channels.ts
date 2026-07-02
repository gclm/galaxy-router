import { apiClient } from './client'
import type {
  Channel,
  CreateChannelRequest,
  FetchModelsRequest,
  PaginatedResponse,
  TestEndpointRequest,
  TestEndpointResponse,
  UpdateChannelRequest,
} from './types'

export interface ChannelListParams {
  search?: string
  status?: string
  sort_by?: string
  sort_order?: string
  page?: number
  page_size?: number
}

export const channelsApi = {
  list: (params?: ChannelListParams) =>
    apiClient.get<PaginatedResponse<Channel>>('/channels', params as Record<string, string | number | undefined>),

  get: (id: string) => apiClient.get<Channel>(`/channels/${id}`),

  create: (data: CreateChannelRequest) =>
    apiClient.post<Channel>('/channels', data),

  update: (id: string, data: UpdateChannelRequest) =>
    apiClient.put<Channel>(`/channels/${id}`, data),

  delete: (id: string) => apiClient.delete<void>(`/channels/${id}`),

  fetchModels: (data: FetchModelsRequest) =>
    apiClient.post<string[]>('/models/fetch', data),

  testEndpoint: (data: TestEndpointRequest) =>
    apiClient.post<TestEndpointResponse>('/channels/test-endpoint', data),
}
