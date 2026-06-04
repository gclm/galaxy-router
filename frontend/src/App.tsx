import { useEffect, useState } from 'react'
import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom'
import { useAuthStore } from '@/stores/auth'
import { getHealth } from '@/api/auth'
import { Layout } from '@/components/layout'
import { Login, Setup, Dashboard, Channels, Groups, ApiKeys, ApiKeyStats, ModelStats, ChannelStats, Settings, Logs, Models, Playground } from '@/pages'

function ProtectedRoute({ children }: { children: React.ReactNode }) {
  const { isAuthenticated, isLoading } = useAuthStore()

  if (isLoading) {
    return (
      <div className="flex h-screen items-center justify-center">
        加载中...
      </div>
    )
  }

  if (!isAuthenticated) {
    return <Navigate to="/login" replace />
  }

  return <>{children}</>
}

function App() {
  const { checkAuth } = useAuthStore()
  const [needsSetup, setNeedsSetup] = useState<boolean | null>(null)

  useEffect(() => {
    const init = async () => {
      try {
        const { needs_setup } = await getHealth()
        setNeedsSetup(needs_setup)

        if (!needs_setup) {
          await checkAuth()
        }
      } catch {
        setNeedsSetup(false)
        await checkAuth()
      }
    }
    init()
  }, [checkAuth])

  if (needsSetup === null) {
    return (
      <div className="flex h-screen items-center justify-center">
        加载中...
      </div>
    )
  }

  if (needsSetup) {
    return (
      <BrowserRouter>
        <Routes>
          <Route path="*" element={<Setup />} />
        </Routes>
      </BrowserRouter>
    )
  }

  return (
    <BrowserRouter>
      <Routes>
        <Route path="/login" element={<Login />} />
        <Route
          path="/"
          element={
            <ProtectedRoute>
              <Layout />
            </ProtectedRoute>
          }
        >
          <Route index element={<Dashboard />} />
          <Route path="channels" element={<Channels />} />
          <Route path="groups" element={<Groups />} />
          <Route path="api-keys" element={<ApiKeys />} />
          <Route path="api-key-stats" element={<ApiKeyStats />} />
          <Route path="stats/models" element={<ModelStats />} />
          <Route path="stats/channels" element={<ChannelStats />} />
          <Route path="logs" element={<Logs />} />
          <Route path="playground" element={<Playground />} />
          <Route path="models" element={<Models />} />
          <Route path="settings" element={<Settings />} />
        </Route>
      </Routes>
    </BrowserRouter>
  )
}

export default App
