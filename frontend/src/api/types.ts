// Auth types
export interface InitRequest {
  username: string
  password: string
  site_title?: string
}

export interface LoginRequest {
  username: string
  password: string
}

export interface AuthResponse {
  token: string
  expires_in: number
}

export interface UserInfoResponse {
  id: string
  username: string
}

export interface ChangePasswordRequest {
  old_password: string
  new_password: string
}

// Channel types
export type EndpointType =
  | 'openai_chat'
  | 'openai_response'
  | 'anthropic'
  | 'gemini'
  | 'openai_embedding'
  | 'openai_images'

export const ENDPOINT_LABELS: Record<EndpointType, string> = {
  openai_chat: 'OpenAI Chat',
  openai_response: 'OpenAI Responses',
  anthropic: 'Anthropic',
  gemini: 'Gemini（仅探测）',
  openai_embedding: 'OpenAI Embedding',
  openai_images: 'OpenAI Images',
}

export interface EndpointConfig {
  type: EndpointType
  base_url: string
  enabled?: boolean
}

export interface CustomHeader {
  key: string
  value: string
}

export interface UpstreamApiKey {
  key: string
  note?: string
  enabled?: boolean
}

export interface Channel {
  id: string
  name: string
  api_keys: UpstreamApiKey[]
  endpoints: EndpointConfig[]
  models: string[]
  rate_limit_rpm: number | null
  rate_limit_tpm: number | null
  failure_threshold: number
  blacklist_minutes: number
  concurrency: number
  custom_headers: CustomHeader[]
  enabled: boolean
  created_at: string
  updated_at: string
}

export interface CreateChannelRequest {
  name: string
  api_keys: UpstreamApiKey[]
  endpoints: EndpointConfig[]
  models?: string[]
  rate_limit_rpm?: number
  rate_limit_tpm?: number
  failure_threshold?: number
  blacklist_minutes?: number
  concurrency?: number
  custom_headers?: CustomHeader[]
  enabled?: boolean
}

export interface UpdateChannelRequest {
  name?: string
  api_keys?: UpstreamApiKey[]
  endpoints?: EndpointConfig[]
  models?: string[]
  rate_limit_rpm?: number
  rate_limit_tpm?: number
  failure_threshold?: number
  blacklist_minutes?: number
  concurrency?: number
  custom_headers?: CustomHeader[]
  enabled?: boolean
}

export interface FetchModelsRequest {
  endpoints: EndpointConfig[]
  api_key: string
}

export interface TestChannelRequest {
  model: string
  test_protocol: string
  api_key: string
  stream?: boolean
  user_agent?: string
}

export interface TestChannelResponse {
  success: boolean
  message: string
  latency_ms: number
  time_to_first_token_ms?: number
  input_prompt: string
  output_content: string | null
  prompt_tokens?: number
  completion_tokens?: number
}

// Group types
export interface GroupItem {
  id: string
  channel_id: string
  model_name: string
  priority: number
  weight: number
}

export interface Group {
  id: string
  name: string
  match_regex: string | null
  retry_enabled: boolean
  max_retries: number
  first_token_timeout_secs: number
  enabled: boolean
  items: GroupItem[]
  created_at: string
  updated_at: string
}

export interface CreateGroupRequest {
  name: string
  match_regex?: string
  retry_enabled?: boolean
  max_retries?: number
  first_token_timeout_secs?: number
  enabled?: boolean
  items: CreateGroupItemRequest[]
}

export interface CreateGroupItemRequest {
  channel_id: string
  model_name: string
  priority?: number
  weight?: number
}

export interface UpdateGroupRequest {
  name?: string
  match_regex?: string
  retry_enabled?: boolean
  max_retries?: number
  first_token_timeout_secs?: number
  enabled?: boolean
  items?: CreateGroupItemRequest[]
}

export interface AddGroupItemRequest {
  channel_id: string
  model_name: string
  priority?: number
  weight?: number
}

// API Key types
export interface ApiKey {
  id: string
  name: string
  api_key: string
  enabled: boolean
  supported_models: string | null
  created_at: string
  updated_at: string
}

export interface CreateApiKeyRequest {
  name: string
  supported_models?: string
}

export interface UpdateApiKeyRequest {
  name?: string
  enabled?: boolean
  supported_models?: string
}

// 分页响应
export interface PaginatedResponse<T> {
  items: T[]
  total: number
}

// Stats types
export interface StatsOverview {
  total_requests: number
  total_input_tokens: number
  total_output_tokens: number
  total_cost: number
  today_requests: number
  today_input_tokens: number
  today_output_tokens: number
  today_cost: number
}

export interface DailyStats {
  date: string
  request_count: number
  success_count: number
  failure_count: number
  input_tokens: number
  output_tokens: number
  cache_read_tokens: number
  cache_creation_tokens: number
  total_cost: number
}

export interface ModelStats {
  model: string
  request_count: number
  input_tokens: number
  output_tokens: number
  total_cost: number
}

export interface ChannelStats {
  channel_id: string
  channel_name: string
  request_count: number
  success_count: number
  failure_count: number
  input_tokens: number
  output_tokens: number
  total_cost: number
}

// System Info types
export interface SystemInfo {
  version: string
  uptime_secs: number
  channel_count: number
  group_count: number
  api_key_count: number
}

// Settings types
export interface SettingItem {
  key: string
  category: string
  value: string
  description: string | null
}

export interface InfraConfig {
  server: { host: string; port: number }
  database: { path: string }
  logging: { level: string; format: string; file: boolean; file_path: string }
  auth: { token_expiry_hours: number }
}

// Request Log types
export interface ChannelAttempt {
  channel_id: string
  channel_name: string | null
  status: string
  duration_ms: number
  error: string | null
  upstream_key_hint: string | null
}

export interface RequestLog {
  id: string
  api_key_id: string | null
  api_key_name: string | null
  channel_id: string | null
  channel_name: string | null
  group_id: string | null
  requested_model: string
  actual_model: string | null
  input_tokens: number
  output_tokens: number
  cache_read_tokens: number
  cache_creation_tokens: number
  cost: number | null
  latency_ms: number | null
  ttft_ms: number | null
  status_code: number | null
  error_message: string | null
  created_at: string
  endpoint_type: string | null
  request_type: string
  is_stream: boolean
  upstream_key_hint: string | null
  attempts: ChannelAttempt[] | null
  user_agent: string | null
}

export interface RequestLogDetail extends RequestLog {
  request_content: string | null
  response_content: string | null
}
