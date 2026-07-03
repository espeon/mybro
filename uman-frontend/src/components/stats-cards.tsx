import { useEffect, useState, useCallback } from "react"
import { api, type StatsResponse } from "@/lib/api"
import { MetricCard } from "./metric-card"

function computeUptime(records: { ts_ms: number; duration_ms: number; status: number }[]): number {
  if (records.length === 0) return 1
  const failed = records.filter((r) => r.status >= 500).length
  return 1 - failed / records.length
}

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
  const throughput = buckets.map((b) => ((b.tokens_in + b.tokens_out) / 60))
  const tokens = buckets.map((b) => b.tokens_in + b.tokens_out)

  const totalTokens = data.summary.tokens_in + data.summary.tokens_out
  const avgTtft = data.summary.avg_ttft_ms
  const uptime = computeUptime(buckets.flatMap((b) =>
    Array.from({ length: Math.min(b.count, 1) }).map(() => ({
      ts_ms: b.ts_ms,
      duration_ms: b.avg_latency_ms,
      status: b.errors > 0 ? 500 : 200,
    }))
  ))

  const avgThroughput =
    throughput.length > 0 ? throughput.reduce((a, b) => a + b, 0) / throughput.length : 0

  return (
    <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
      <MetricCard
        title="Uptime"
        value={`${(uptime * 100).toFixed(3)}%`}
        subtitle="last 1h"
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
        sparkline={throughput.length > 0 ? throughput : [0]}
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
