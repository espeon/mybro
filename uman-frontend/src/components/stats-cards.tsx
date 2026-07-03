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
  const latencies = buckets.map((b) => b.p95_latency_ms)
  const throughput = buckets.map((b) => ((b.tokens_in + b.tokens_out) / 60))
  const tokens = buckets.map((b) => b.tokens_in + b.tokens_out)

  const totalTokens = data.summary.tokens_in + data.summary.tokens_out
  const avgLatency = data.summary.avg_latency_ms
  const maxLatency = buckets.reduce((m, b) => Math.max(m, b.max_latency_ms), 0)
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
        title="Latency"
        value={`${avgLatency.toFixed(0)}ms`}
        subtitle={`p95 ${maxLatency}ms · last 1h`}
        sparkline={latencies.length > 0 ? latencies : [0]}
        color="amber"
      />
      <MetricCard
        title="Throughput"
        value={`${avgThroughput.toFixed(1)} tok/s`}
        subtitle="tok/s avg · last 1h"
        sparkline={throughput.length > 0 ? throughput : [0]}
        color="cyan"
      />
      <MetricCard
        title="Tokens"
        value={totalTokens > 0 ? `${(totalTokens / 1000).toFixed(1)}k` : "0"}
        subtitle="in + out · last 1h"
        sparkline={tokens.length > 0 ? tokens : [0]}
        color="primary"
      />
    </div>
  )
}
