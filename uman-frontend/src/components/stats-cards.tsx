import { useEffect, useState, useCallback } from "react"
import { api, type StatsResponse } from "@/lib/api"
import { MetricCard } from "./metric-card"

export function StatsCards() {
  const [data, setData] = useState<StatsResponse | null>(null)
  const [error, setError] = useState<string | null>(null)

  const refresh = useCallback(async () => {
    try {
      setError(null)
      const d = await api.getStats(3600, 60)
      setData(d)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }, [])

  useEffect(() => {
    refresh()
    const interval = setInterval(refresh, 15000)
    return () => clearInterval(interval)
  }, [refresh])

  if (!data && !error) return null
  if (error) return <p className="text-sm text-destructive">{error}</p>
  if (!data) return null

  const buckets = data.buckets.length > 0 ? data.buckets : []
  const counts = buckets.map((b) => b.count)
  const ttfts = buckets.map((b) => b.p50_ttft_ms)
  const tokens = buckets.map((b) => b.tokens_in + b.tokens_out)
  const avgTtft = data.summary.avg_ttft_ms

  // Uptime: fraction of successful (non-5xx) requests in the window
  const totalCount = data.summary.count
  const totalErrors = data.summary.errors
  const uptime = totalCount > 0 ? 1 - totalErrors / totalCount : 1
  const uptimeDisplay = totalCount === 0 ? "—" : `${(uptime * 100).toFixed(1)}%`

  // Throughput: total tokens / window seconds, ignoring empty buckets
  const totalTokens = data.summary.tokens_in + data.summary.tokens_out
  // The stats window is "last 1h" (3600s) — use the actual window, not bucket count
  const windowSeconds = 3600
  const avgThroughput = windowSeconds > 0 ? totalTokens / windowSeconds : 0

  return (
    <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
      <MetricCard
        title="Uptime"
        value={uptimeDisplay}
        subtitle={`${totalCount.toLocaleString()} reqs · last 1h`}
        sparkline={counts.length > 0 ? counts : [0]}
        color="emerald"
      />
      <MetricCard
        title="TTFT"
        value={`${avgTtft.toFixed(0)}ms`}
        subtitle="time to first token · last 1h"
        sparkline={ttfts.length > 0 ? ttfts : [0]}
        color="cyan"
      />
      <MetricCard
        title="Throughput"
        value={`${avgThroughput.toFixed(1)} tok/s`}
        subtitle="tok/s avg · last 1h"
        sparkline={tokens.length > 0 ? tokens : [0]}
        color="primary"
      />
      <MetricCard
        title="Tokens"
        value={totalTokens > 0 ? `${(totalTokens / 1000).toFixed(1)}k` : "0"}
        subtitle="in + out · last 1h"
        sparkline={tokens.length > 0 ? tokens : [0]}
        color="rose"
      />
    </div>
  )
}
