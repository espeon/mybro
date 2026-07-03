import { HealthCard } from "@/components/health-card"
import { KeysCard } from "@/components/keys-card"
import { ModelsCard } from "@/components/models-card"
import { StatsCards } from "@/components/stats-cards"
import { StatsChart } from "@/components/stats-chart"
import { TokenStatsCard } from "@/components/token-stats-card"
import { useHealth } from "@/hooks/use-health"
import { Button } from "@/components/ui/button"
import { api } from "@/lib/api"
import { useEffect, useState } from "react"

export function App() {
  const { health, error, refresh } = useHealth()
  const [apiKey, setApiKey] = useState("")

  useEffect(() => {
    setApiKey(localStorage.getItem("umans_api_key") || "")
  }, [])

  const saveKey = (value: string) => {
    setApiKey(value)
    if (value) {
      localStorage.setItem("umans_api_key", value)
    } else {
      localStorage.removeItem("umans_api_key")
    }
  }

  const handleRestart = async () => {
    try {
      await api.restart()
    } catch {
      // expected — the server exits
    }
  }

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
            <Button variant="outline" size="sm" onClick={refresh}>
              Refresh
            </Button>
            <Button variant="destructive" size="sm" onClick={handleRestart}>
              Restart
            </Button>
          </div>
        </div>

        {/* API Key input */}
        <div className="flex items-center gap-2">
          <input
            type="password"
            placeholder="Dashboard API key (optional if no keys set)"
            value={apiKey}
            onChange={(e) => saveKey(e.target.value)}
            className="h-9 flex-1 rounded-md border border-input bg-transparent px-3 text-sm shadow-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          />
        </div>

        {error && (
          <div className="rounded-md border border-destructive bg-destructive/10 p-2 text-sm text-destructive">
            {error}
          </div>
        )}

        {/* Metric cards with sparklines */}
        <StatsCards />

        {/* Time-series chart */}
        <StatsChart />

        {/* Cards */}
        <div className="grid gap-4 md:grid-cols-2">
          <HealthCard health={health} />
          <KeysCard />
        </div>

        <div className="grid gap-4 md:grid-cols-2">
          <ModelsCard />
          <TokenStatsCard />
        </div>
      </div>
    </div>
  )
}

export default App
