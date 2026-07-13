import { useEffect, useState } from 'react'
import { BrowserRouter, Routes, Route, Navigate, useLocation } from 'react-router-dom'
import { useAuthStore } from '@/stores/auth'
import { getHealth } from '@/api/auth'
import { Layout } from '@/components/layout'
import { ErrorBoundary } from '@/components/ErrorBoundary'
import { Login, Setup, Dashboard, Channels, RoutesPage, ApiKeys, ApiKeyStats, ModelStats, ChannelStats, Settings, ClientConfig, Logs, Models, Playground, NotFound } from '@/pages'

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

/** 页面切换时顶部显示进度条（兼容 BrowserRouter） */
function NavigationProgress() {
  const location = useLocation()
  const [active, setActive] = useState(false)
  const [prevPathname, setPrevPathname] = useState(location.pathname)

  useEffect(() => {
    if (location.pathname !== prevPathname) {
      // eslint-disable-next-line react-hooks/set-state-in-effect -- 响应路由变化的进度条副作用，动画时机依赖 state 切换，非派生可表达
      setActive(true)
      setPrevPathname(location.pathname)
      const timer = setTimeout(() => setActive(false), 300)
      return () => clearTimeout(timer)
    }
  }, [location.pathname, prevPathname])

  return (
    <div className="fixed top-0 left-0 right-0 z-[9999] h-0.5">
      {active && (
        <div className="h-full bg-primary animate-[nav-progress_300ms_ease-out_forwards]" />
      )}
    </div>
  )
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
        <ErrorBoundary>
          <Routes>
            <Route path="*" element={<Setup />} />
          </Routes>
        </ErrorBoundary>
      </BrowserRouter>
    )
  }

  return (
    <BrowserRouter>
      <ErrorBoundary>
        <NavigationProgress />
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
            <Route path="routes" element={<RoutesPage />} />
            <Route path="api-keys" element={<ApiKeys />} />
            <Route path="api-key-stats" element={<ApiKeyStats />} />
            <Route path="stats/models" element={<ModelStats />} />
            <Route path="stats/channels" element={<ChannelStats />} />
            <Route path="logs" element={<Logs />} />
            <Route path="playground" element={<Playground />} />
            <Route path="models" element={<Models />} />
            <Route path="settings" element={<Settings />} />
            <Route path="client-config" element={<ClientConfig />} />
          </Route>
          <Route path="*" element={<NotFound />} />
        </Routes>
      </ErrorBoundary>
    </BrowserRouter>
  )
}

export default App
