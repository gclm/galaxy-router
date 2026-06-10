import { apiClient } from './client'

export interface ModelInfo {
  model: string
  provider: string
  mode: string
  input_price: number | null
  output_price: number | null
  cache_read_price: number | null
  cache_creation_price: number | null
  max_input_tokens: number | null
  max_output_tokens: number | null
  supports_function_calling: boolean | null
  supports_reasoning: boolean | null
  supports_vision: boolean | null
  supports_pdf_input: boolean | null
  supports_prompt_caching: boolean | null
  supports_system_messages: boolean | null
  supports_tool_choice: boolean | null
}

export const modelInfoApi = {
  list: () => apiClient.get<ModelInfo[]>('/models'),

  get: (model: string) => apiClient.get<ModelInfo>(`/models/${encodeURIComponent(model)}`),

  update: (data: Partial<ModelInfo> & { model: string }) => apiClient.put<void>('/models', data),
}
