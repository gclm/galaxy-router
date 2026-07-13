import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { channelsApi } from '@/api/channels'
import { routesApi } from '@/api/routes'
import { apiKeysApi } from '@/api/api-keys'
import { statsApi } from '@/api/stats'
import { modelInfoApi, type ModelInfo } from '@/api/model-info'
import { settingsApi } from '@/api/settings'
import { backupApi, type BackupFile } from '@/api/backup'
import { apiClient } from '@/api/client'
import type {
  CreateChannelRequest,
  UpdateChannelRequest,
  CreateRouteRequest,
  UpdateRouteRequest,
  CreateApiKeyRequest,
  UpdateApiKeyRequest,
  SystemInfo,
  UpdateCheck,
  ChangePasswordRequest,
} from '@/api/types'

// ─── Channels ────────────────────────────────────────────

export function useChannels() {
  return useQuery({ queryKey: ['channels'], queryFn: () => channelsApi.list() })
}

export function useCreateChannel() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (data: CreateChannelRequest) => channelsApi.create(data),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['channels'] }),
  })
}

export function useUpdateChannel() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ id, data }: { id: string; data: UpdateChannelRequest }) => channelsApi.update(id, data),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['channels'] }),
  })
}

export function useDeleteChannel() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => channelsApi.delete(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['channels'] }),
  })
}

// ─── Routes ──────────────────────────────────────────────

export function useRoutes() {
  return useQuery({ queryKey: ['routes'], queryFn: () => routesApi.list() })
}

export function useCreateRoute() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (data: CreateRouteRequest) => routesApi.create(data),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['routes'] }),
  })
}

export function useUpdateRoute() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ id, data }: { id: string; data: UpdateRouteRequest }) => routesApi.update(id, data),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['routes'] }),
  })
}

export function useDeleteRoute() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => routesApi.delete(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['routes'] }),
  })
}

// ─── API Keys ────────────────────────────────────────────

export function useApiKeys() {
  return useQuery({ queryKey: ['api-keys'], queryFn: () => apiKeysApi.list() })
}

export function useCreateApiKey() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (data: CreateApiKeyRequest) => apiKeysApi.create(data),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['api-keys'] }),
  })
}

export function useUpdateApiKey() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ id, data }: { id: string; data: UpdateApiKeyRequest }) => apiKeysApi.update(id, data),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['api-keys'] }),
  })
}

export function useDeleteApiKey() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => apiKeysApi.delete(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['api-keys'] }),
  })
}

// ─── Stats ───────────────────────────────────────────────

export function useStatsOverview() {
  return useQuery({
    queryKey: ['stats', 'overview'],
    queryFn: () => statsApi.overview(),
    staleTime: 60_000,
  })
}

export function useSystemInfo() {
  return useQuery({
    queryKey: ['system-info'],
    queryFn: () => apiClient.get<SystemInfo>('/system-info'),
    staleTime: 5 * 60_000,
  })
}

export function useUpdateCheck() {
  return useQuery({
    queryKey: ['update-check'],
    queryFn: () => apiClient.get<UpdateCheck>('/update-check'),
    refetchInterval: 60 * 60_000, // 1 小时轮询
    refetchOnMount: 'always',
    retry: false, // 国内 GitHub 不稳，失败不重试避免雪上加霜
  })
}

export function useStatsDaily(params?: { days?: number; start_date?: string; end_date?: string }) {
  return useQuery({
    queryKey: ['stats', 'daily', params],
    queryFn: () => statsApi.daily(params),
    staleTime: 30_000,
  })
}

export function useStatsModels(params?: { days?: number; start_date?: string; end_date?: string }) {
  return useQuery({
    queryKey: ['stats', 'models', params],
    queryFn: () => statsApi.models(params),
    staleTime: 30_000,
  })
}

export function useStatsChannels(params?: { days?: number; start_date?: string; end_date?: string }) {
  return useQuery({
    queryKey: ['stats', 'channels', params],
    queryFn: () => statsApi.channels(params),
    staleTime: 30_000,
  })
}

export function useStatsLatency(params?: { days?: number; start_date?: string; end_date?: string }) {
  return useQuery({
    queryKey: ['stats', 'latency', params],
    queryFn: () => statsApi.latency(params),
    staleTime: 30_000,
  })
}

export function useStatsApiKeys(days?: number) {
  return useQuery({
    queryKey: ['stats', 'api-keys', days],
    queryFn: () => apiClient.get<Array<{
      api_key_id: string
      api_key_name: string
      request_count: number
      success_count: number
      failure_count: number
      input_tokens: number
      output_tokens: number
      total_cost: number
    }>>(`/stats/api-keys${days ? `?days=${days}` : ''}`),
    staleTime: 30_000,
  })
}

export function useBudgets() {
  return useQuery({ queryKey: ['budgets'], queryFn: () => statsApi.listBudgets() })
}

export function useSetBudget() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (data: Parameters<typeof statsApi.setBudget>[0]) => statsApi.setBudget(data),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['budgets'] }),
  })
}

export function useDeleteBudget() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => statsApi.deleteBudget(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['budgets'] }),
  })
}

// ─── Logs ────────────────────────────────────────────────

export function useLogs(params?: Record<string, string | number | undefined>) {
  return useQuery({
    queryKey: ['logs', params],
    queryFn: () => statsApi.logs(params),
    staleTime: 10_000,
  })
}

export function useLogDetail(id: string | null) {
  return useQuery({
    queryKey: ['logs', id],
    queryFn: () => statsApi.logDetail(id!),
    enabled: !!id,
  })
}

export function useLogModels() {
  return useQuery({ queryKey: ['logs', 'models'], queryFn: () => statsApi.logModels() })
}

// ─── Models ──────────────────────────────────────────────

export function useModels() {
  return useQuery({ queryKey: ['models'], queryFn: () => modelInfoApi.list() })
}

export function useUpdateModel() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (data: Partial<ModelInfo> & { model: string }) => modelInfoApi.update(data),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['models'] }),
  })
}

// ─── Settings ────────────────────────────────────────────

export function useSettings() {
  return useQuery({ queryKey: ['settings'], queryFn: () => settingsApi.list() })
}

export function useInfraConfig() {
  return useQuery({ queryKey: ['settings', 'infra'], queryFn: () => settingsApi.infra() })
}

export function useUpdateSetting() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ key, value }: { key: string; value: string }) => settingsApi.update(key, value),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['settings'] }),
  })
}

export function useChangePassword() {
  return useMutation({
    mutationFn: (data: ChangePasswordRequest) => apiClient.put('/auth/password', data),
  })
}

// ─── Backup ──────────────────────────────────────────────

export function useExportBackup() {
  return useMutation({ mutationFn: () => backupApi.export() })
}

export function useImportBackup() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (data: BackupFile) => backupApi.import(data),
    onSuccess: () => qc.invalidateQueries(),
  })
}

export function useResetBackup() {
  return useMutation({ mutationFn: () => backupApi.reset() })
}
