import { HealthCard } from "@/components/health-card"
import { KeysCard } from "@/components/keys-card"
import { ModelsCard } from "@/components/models-card"
import { StatsCards } from "@/components/stats-cards"
import { StatsChart } from "@/components/stats-chart"
import { TokenStatsCard } from "@/components/token-stats-card"
import { CacheCard } from "@/components/cache-card"
import { InFlightCard } from "@/components/in-flight-card"
import { RecentRequestsTable } from "@/components/recent-requests-table"
import { StatusBar } from "@/components/status-bar"
import { LoginPage } from "@/components/login-page"
import { useStatsSummary } from "@/hooks/use-stats-summary"
import { useGate } from "@/hooks/use-gate"
import { useRecentRequests } from "@/hooks/use-recent-requests"
import { Button } from "@/components/ui/button"
import { api } from "@/lib/api"
import { useEffect, useState } from "react"

function useAuth() {
  const [authed, setAuthed] = useState(() => !!localStorage.getItem("umans_api_key"))
  const login = () => setAuthed(true)
  const logout = () => {
    localStorage.removeItem("umans_api_key")
    setAuthed(false)
  }
  return { authed, login, logout }
}

export function App() {
  const { authed, login, logout } = useAuth()
  const [apiKey, setApiKey] = useState("")

  useEffect(() => {
    setApiKey(localStorage.getItem("umans_api_key") || "")
  }, [authed])

  const handleRestart = async () => {
    try {
      await api.restart()
    } catch {
      // expected — the server exits
    }
  }

  // First, check whether the server actually requires auth. If API_KEYS is empty,
  // /api/validate returns ok without a key, so we can skip login.
  // - 200 + valid:true  → auth disabled OR our stored key works → no login needed
  // - 200 + valid:false → auth is required, no key → show login
  // - 401               → auth is required, our key was rejected → show login
  // - network error     → retry; do NOT bypass auth (server might be up but unreachable)
  const [needsAuth, setNeedsAuth] = useState<boolean | null>(null)
  const [authCheckFailed, setAuthCheckFailed] = useState(false)
  useEffect(() => {
    let cancelled = false
    const check = async () => {
      try {
        const resp = await fetch("/api/validate", {
          headers: { "X-Api-Key": localStorage.getItem("umans_api_key") || "" },
        })
        if (cancelled) return
        if (resp.status === 401) {
          setNeedsAuth(true)
        } else {
          const j = await resp.json().catch(() => ({}))
          setNeedsAuth(j.valid === false)
        }
        setAuthCheckFailed(false)
      } catch {
        if (cancelled) return
        // Network error: leave needsAuth as null (still loading) and surface the error
        setAuthCheckFailed(true)
      }
    }
    check()
    return () => {
      cancelled = true
    }
  }, [])

  if (needsAuth === null) {
    if (authCheckFailed) {
      return (
        <div className="min-h-svh bg-background flex items-center justify-center p-4">
          <div className="text-center space-y-2">
            <h1 className="text-lg font-semibold">mybro</h1>
            <p className="text-sm text-muted-foreground">
              Can&apos;t reach the proxy. Is it running?
            </p>
          </div>
        </div>
      )
    }
    // Still figuring out if auth is needed — render nothing
    return <div className="min-h-svh bg-background" />
  }

  if (needsAuth && !authed) {
    return <LoginPage onSuccess={login} />
  }

  return (
    <Dashboard
      apiKey={apiKey}
      onLogout={logout}
      onRestart={handleRestart}
    />
  )
}

function Dashboard({
  apiKey,
  onLogout,
  onRestart,
}: {
  apiKey: string
  onLogout: () => void
  onRestart: () => void
}) {
  const { summary, error: summaryError } = useStatsSummary(3600)
  const { gate, error: gateError } = useGate()
  const { records } = useRecentRequests(50)

  return (
    <div className="min-h-svh bg-background p-4 md:p-6">
      <div className="mx-auto max-w-5xl space-y-4">
        {/* Header */}
        <div className="flex items-center justify-between">
          <div>
            <h1 className="text-lg font-semibold">mybro</h1>
            <p className="text-xs text-muted-foreground">local reverse proxy & dashboard</p>
          </div>
          <div className="flex items-center gap-2">
            <Button variant="outline" size="sm" onClick={() => location.reload()}>
              Refresh
            </Button>
            <Button variant="ghost" size="sm" onClick={onLogout}>
              Sign out
            </Button>
            <Button variant="destructive" size="sm" onClick={onRestart}>
              Restart
            </Button>
          </div>
        </div>

        {/* Status bar — dot color reflects actual health, not just "signed in" */}
        <StatusBar apiKey={apiKey} gate={gate} />

        {/* Metric cards (no Latency — nixed) */}
        <StatsCards />

        {/* In-flight + Cache row */}
        <div className="grid gap-4 md:grid-cols-2">
          {gate && (
            <InFlightCard
              active={gate.active}
              queued={gate.queued}
              limit={gate.limit}
              hardCap={gate.hard_cap}
              maxQueue={gate.max_queue_size}
              throttled={gate.throttled}
              error={gateError}
            />
          )}
          <CacheCard summary={summary} error={summaryError} />
        </div>

        {/* Time-series chart */}
        <StatsChart />

        {/* Recent requests table */}
        <RecentRequestsTable records={records} />

        {/* Health + Keys */}
        <div className="grid gap-4 md:grid-cols-2">
          <HealthCard />
          <KeysCard />
        </div>

        {/* Token stats + Models */}
        <div className="grid gap-4 md:grid-cols-2">
          <TokenStatsCard />
          <ModelsCard />
        </div>
      </div>
    </div>
  )
}

export default App