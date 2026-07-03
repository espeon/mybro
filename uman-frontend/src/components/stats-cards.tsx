import { useEffect, useState, useCallback } from "react"
import { api, type StatsResponse } from "@/lib/api"
import { MetricCard } from "./metric-card"
import { formatNumber, formatTime } from "@/lib/utils"
import { useStatsFilter } from "@/hooks/use-stats-filter"

export function StatsCards() {
  const { window, model } = useStatsFilter()
  const [data, setData] = useState<StatsResponse | null>(null)
  const [error, setError] = useState<string | null>(null)

  const refresh = useCallback(async () => {
    try {
      setError(null)
      const bucket =
        window <= 300
          ? 10
          : window <= 900
            ? 30
            : window <= 3600
              ? 60
              : window <= 21600
                ? 600
                : 3600
      const d = await api.getStats(window, bucket, "buckets", model)
      setData(d)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }, [window, model])

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
  const tokensOut = buckets.map((b) => b.tokens_out)
  const avgTtft = data.summary.avg_ttft_ms

  // Uptime: fraction of successful (non-5xx) requests in the window
  const totalCount = data.summary.count
  const totalErrors = data.summary.errors
  const uptime = totalCount > 0 ? 1 - totalErrors / totalCount : 1
  const uptimeDisplay = totalCount === 0 ? "—" : `${(uptime * 100).toFixed(1)}%`

  // Throughput: output tokens generated per second of actual generation time
  const totalTokens = data.summary.tokens_in + data.summary.tokens_out
  const genSeconds = data.summary.gen_time_ms / 1000
  const avgThroughput =
    genSeconds > 0 ? data.summary.gen_tokens_out / genSeconds : 0

  const windowLabel =
    window >= 3600 ? `${window / 3600}h` : `${window / 60}m`

  return (
    <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
      <MetricCard
        title="Uptime"
        value={uptimeDisplay}
        subtitle={`${totalCount.toLocaleString()} reqs · last ${windowLabel}`}
        sparkline={counts.length > 0 ? counts : [0]}
        color="emerald"
      />
      <MetricCard
        title="TTFT"
        value={formatTime(avgTtft)}
        subtitle={`time to first token · last ${windowLabel}`}
        sparkline={ttfts.length > 0 ? ttfts : [0]}
        color="cyan"
      />
      <MetricCard
        title="Throughput"
        value={`${formatNumber(avgThroughput)} tok/s`}
        subtitle={`avg of last ${windowLabel}`}
        sparkline={tokensOut.length > 0 ? tokensOut : [0]}
        color="primary"
      />
      <MetricCard
        title="Tokens"
        value={totalTokens > 0 ? formatNumber(totalTokens) : "0"}
        subtitle={`in + out · last ${windowLabel}`}
        sparkline={tokens.length > 0 ? tokens : [0]}
        color="rose"
      />
    </div>
  )
}
