const API_BASE = '/api/v1/admin'

interface ApiResponse<T> {
  code: number
  message: string
  data?: T
}

class ApiClient {
  private token: string | null = null

  setToken(token: string | null) {
    this.token = token
    if (token) {
      localStorage.setItem('token', token)
    } else {
      localStorage.removeItem('token')
    }
  }

  getToken(): string | null {
    if (!this.token) {
      this.token = localStorage.getItem('token')
    }
    return this.token
  }

  private async request<T>(
    path: string,
    options: RequestInit = {}
  ): Promise<T> {
    const token = this.getToken()
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
      ...((options.headers as Record<string, string>) || {}),
    }

    if (token) {
      headers['Authorization'] = `Bearer ${token}`
    }

    const response = await fetch(`${API_BASE}${path}`, {
      ...options,
      headers,
    })

    if (response.status === 401) {
      this.setToken(null)
      window.location.href = '/login'
      throw new Error('未授权')
    }

    const text = await response.text()
    let data: ApiResponse<T>
    try {
      data = JSON.parse(text) as ApiResponse<T>
    } catch {
      throw new Error(text || `HTTP ${response.status}`)
    }

    if (!response.ok || data.code !== 0) {
      throw new Error(data.message || 'Request failed')
    }

    return data.data as T
  }

  async get<T>(path: string, params?: Record<string, string | number | undefined>): Promise<T> {
    if (params) {
      const searchParams = new URLSearchParams()
      Object.entries(params).forEach(([key, value]) => {
        if (value !== undefined && value !== '') {
          searchParams.set(key, String(value))
        }
      })
      const qs = searchParams.toString()
      if (qs) {
        return this.request<T>(`${path}?${qs}`)
      }
    }
    return this.request<T>(path)
  }

  async post<T>(path: string, body?: unknown): Promise<T> {
    return this.request<T>(path, {
      method: 'POST',
      body: body ? JSON.stringify(body) : undefined,
    })
  }

  async put<T>(path: string, body?: unknown): Promise<T> {
    return this.request<T>(path, {
      method: 'PUT',
      body: body ? JSON.stringify(body) : undefined,
    })
  }

  async delete<T>(path: string): Promise<T> {
    return this.request<T>(path, {
      method: 'DELETE',
    })
  }
}

export const apiClient = new ApiClient()
