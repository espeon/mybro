import { useEffect, useState, useCallback, useRef } from "react"
import {
  Area,
  AreaChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts"
import { api, type StatsResponse } from "@/lib/api"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Button } from "@/components/ui/button"
import { useStatsFilter } from "@/hooks/use-stats-filter"
import type { ValueType } from "recharts/types/component/DefaultTooltipContent"

type Metric = "count" | "latency" | "errors" | "tokens"

export function StatsChart() {
  const { window: win, model } = useStatsFilter()
  const [data, setData] = useState<StatsResponse | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [metric, setMetric] = useState<Metric>("count")
  const refreshRef = useRef<(() => void) | null>(null)

  const refresh = useCallback(async () => {
    try {
      setError(null)
      const bucket =
        win <= 300
          ? 10
          : win <= 900
            ? 30
            : win <= 3600
              ? 60
              : win <= 21600
                ? 600
                : 3600
      const d = await api.getStats(win, bucket, "buckets", model)
      setData(d)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }, [win, model])

  refreshRef.current = refresh

  useEffect(() => {
    refresh()
    const interval = setInterval(() => refreshRef.current?.(), 5000)
    return () => clearInterval(interval)
  }, [refresh])

  const buckets = data?.buckets ?? []
  const summary = data?.summary

  // Transform buckets into recharts format
  const chartData = buckets.map((b) => {
    const d = new Date(b.ts_ms)
    return {
      ts: b.ts_ms,
      label: d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }),
      time: d.toLocaleTimeString(),
      count: b.count,
      latency: b.p95_latency_ms,
      errors: b.errors,
      tokens: b.tokens_in + b.tokens_out,
      throttled: b.throttled,
    }
  })

  const metricConfig: Record<
    Metric,
    { key: string; label: string; color: string; format: (v: number) => string }
  > = {
    count: {
      key: "count",
      label: "Requests",
      color: "#10b981",
      format: (v) => v.toString(),
    },
    latency: {
      key: "latency",
      label: "Latency p95",
      color: "#f59e0b",
      format: (v) => `${Math.round(v)}ms`,
    },
    errors: {
      key: "errors",
      label: "Errors",
      color: "#ef4444",
      format: (v) => v.toString(),
    },
    tokens: {
      key: "tokens",
      label: "Tokens",
      color: "#06b6d4",
      format: (v) => (v > 1000 ? `${(v / 1000).toFixed(1)}k` : v.toString()),
    },
  }

  const cfg = metricConfig[metric]

  return (
    <Card>
      <CardHeader>
        <div className="flex flex-wrap items-center justify-between gap-2">
          <CardTitle>Request Activity</CardTitle>
          <div className="flex flex-wrap gap-1">
            {(Object.keys(metricConfig) as Metric[]).map((m) => (
              <Button
                key={m}
                variant={metric === m ? "default" : "outline"}
                size="sm"
                className="h-7 px-2 text-xs"
                onClick={() => setMetric(m)}
              >
                {metricConfig[m].label}
              </Button>
            ))}
          </div>
        </div>
      </CardHeader>
      <CardContent>
        {error && <p className="text-sm text-destructive">{error}</p>}
        {!data && !error && (
          <p className="text-sm text-muted-foreground">Loading...</p>
        )}

        {data && (
          <div className="space-y-3">
            {/* Summary row */}
            <div className="flex flex-wrap gap-3 text-xs">
              <div className="flex items-center gap-1">
                <span className="text-muted-foreground">Total:</span>
                <span className="font-mono font-medium">
                  {summary?.count ?? 0}
                </span>
              </div>
              <div className="flex items-center gap-1">
                <span className="text-muted-foreground">Errors:</span>
                <span className="font-mono font-medium text-destructive">
                  {summary?.errors ?? 0}
                </span>
              </div>
              <div className="flex items-center gap-1">
                <span className="text-muted-foreground">Throttled:</span>
                <span className="font-mono font-medium">
                  {summary?.throttled ?? 0}
                </span>
              </div>
              <div className="flex items-center gap-1">
                <span className="text-muted-foreground">Avg latency:</span>
                <span className="font-mono font-medium">
                  {Math.round(summary?.avg_latency_ms ?? 0)}ms
                </span>
              </div>
              <div className="flex items-center gap-1">
                <span className="text-muted-foreground">Tokens:</span>
                <span className="font-mono font-medium">
                  {(
                    (summary?.tokens_in ?? 0) + (summary?.tokens_out ?? 0)
                  ).toLocaleString()}
                </span>
              </div>
            </div>

            {/* Recharts area chart */}
            <div className="h-48 w-full">
              <ResponsiveContainer width="100%" height="100%">
                <AreaChart
                  data={chartData}
                  margin={{ top: 8, right: 16, left: 0, bottom: 0 }}
                >
                  <defs>
                    <linearGradient
                      id={`gradient-${metric}`}
                      x1="0"
                      y1="0"
                      x2="0"
                      y2="1"
                    >
                      <stop
                        offset="5%"
                        stopColor={cfg.color}
                        stopOpacity={0.4}
                      />
                      <stop
                        offset="95%"
                        stopColor={cfg.color}
                        stopOpacity={0}
                      />
                    </linearGradient>
                  </defs>
                  <CartesianGrid
                    strokeDasharray="3 3"
                    stroke="#374151"
                    opacity={0.3}
                  />
                  <XAxis
                    dataKey="label"
                    tick={{ fontSize: 10, fill: "#9ca3af" }}
                    tickLine={false}
                    axisLine={{ stroke: "#374151" }}
                  />
                  <YAxis
                    tick={{ fontSize: 10, fill: "#9ca3af" }}
                    tickLine={false}
                    axisLine={{ stroke: "#374151" }}
                    width={48}
                  />
                  <Tooltip
                    contentStyle={{
                      backgroundColor: "#1f2937",
                      border: "1px solid #374151",
                      borderRadius: "6px",
                      fontSize: "12px",
                    }}
                    labelStyle={{ color: "#f3f4f6" }}
                    formatter={(value: ValueType | undefined) => [
                      cfg.format(Number(value ?? 0)),
                      cfg.label,
                    ]}
                  />
                  <Area
                    type="monotone"
                    dataKey={cfg.key}
                    stroke={cfg.color}
                    strokeWidth={2}
                    fill={`url(#gradient-${metric})`}
                  />
                </AreaChart>
              </ResponsiveContainer>
            </div>

            {chartData.length === 0 && (
              <p className="py-4 text-center text-sm text-muted-foreground">
                No data in this time window
              </p>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  )
}
