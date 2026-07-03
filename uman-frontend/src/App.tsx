import { HealthCard } from "@/components/health-card"
import { KeysCard } from "@/components/keys-card"
import { ModelsCard } from "@/components/models-card"
import { StatsCards } from "@/components/stats-cards"
import { StatsChart } from "@/components/stats-chart"
import { TokenStatsCard } from "@/components/token-stats-card"
import { CacheCard } from "@/components/cache-card"
import { InFlightCard } from "@/components/in-flight-card"
import { RecentRequestsTable } from "@/components/recent-requests-table"
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
  const [needsAuth, setNeedsAuth] = useState<boolean | null>(null)
  useEffect(() => {
    fetch("/api/validate", { headers: { "X-Api-Key": localStorage.getItem("umans_api_key") || "" } })
      .then((r) => r.json())
      .then((j) => {
        // If the server returned 200 with valid:true, we're in (or auth is disabled).
        // If 200 with valid:false, auth is required but we don't have it.
        // We only need to show login if auth is required AND we don't have a key.
        setNeedsAuth(true)
        if (j.valid === true) {
          // Either auth disabled, or our stored key works
          setNeedsAuth(false)
        }
      })
      .catch(() => setNeedsAuth(false))
  }, [])

  if (needsAuth === null) {
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
  const { summary } = useStatsSummary(3600)
  const { gate } = useGate()
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

        {/* Status bar */}
        <div className="flex items-center justify-between rounded-md border bg-card px-3 py-2 text-xs text-muted-foreground">
          <div className="flex items-center gap-2">
            <span className="inline-block h-2 w-2 rounded-full bg-emerald-500" />
            Signed in as <span className="font-mono text-foreground">…{apiKey.slice(-6)}</span>
          </div>
          {gate && (
            <div className="flex items-center gap-2">
              <span>{gate.active} in-flight</span>
              {gate.queued > 0 && <span className="text-amber-500">{gate.queued} queued</span>}
            </div>
          )}
        </div>

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
            />
          )}
          <CacheCard summary={summary} />
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