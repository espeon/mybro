import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Badge } from "@/components/ui/badge"
import type { RequestRecord } from "@/lib/api"
import { useStatsFilter } from "@/hooks/use-stats-filter"

function formatTime(ts_ms: number): string {
  return new Date(ts_ms).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  })
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`
  return `${(ms / 1000).toFixed(2)}s`
}

function statusVariant(
  status: number
): "default" | "destructive" | "secondary" {
  if (status >= 500) return "destructive"
  if (status >= 400) return "secondary"
  return "default"
}

export function RecentRequestsTable({ records }: { records: RequestRecord[] }) {
  const { setModel } = useStatsFilter()

  return (
    <Card>
      <CardHeader>
        <CardTitle>Recent Requests</CardTitle>
      </CardHeader>
      <CardContent className="h-100 max-h-100 space-y-2 overflow-y-auto">
        {records.length === 0 ? (
          <p className="text-sm text-muted-foreground">No requests yet</p>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-xs">
              <thead>
                <tr className="border-b text-muted-foreground">
                  <th className="py-1 pr-2 text-left font-medium">Time</th>
                  <th className="py-1 pr-2 text-left font-medium">Model</th>
                  <th className="py-1 pr-2 text-left font-medium">Key</th>
                  <th className="py-1 pr-2 text-left font-medium">Status</th>
                  <th className="py-1 pr-2 text-right font-medium">TTFT</th>
                  <th className="py-1 pr-2 text-right font-medium">Total</th>
                  <th className="py-1 pr-2 text-right font-medium">In</th>
                  <th className="py-1 pr-2 text-right font-medium">Out</th>
                  <th className="py-1 pr-2 text-right font-medium">Cache</th>
                  <th className="py-1 pr-2 text-right font-medium">Write</th>
                </tr>
              </thead>
              <tbody>
                {records.slice(0, 30).map((r, i) => (
                  <tr key={i} className="border-b last:border-0">
                    <td className="py-1 pr-2 font-mono text-muted-foreground">
                      {formatTime(r.ts_ms)}
                    </td>
                    <td className="py-1 pr-2 font-mono">
                      <button
                        className="cursor-pointer hover:text-primary hover:underline"
                        onClick={() => setModel(r.model)}
                        title={`Filter by ${r.model}`}
                      >
                        {r.model}
                      </button>
                    </td>
                    <td className="py-1 pr-2 font-mono text-muted-foreground">
                      {r.key_name || "—"}
                    </td>
                    <td className="py-1 pr-2">
                      <Badge variant={statusVariant(r.status)}>
                        {r.status}
                      </Badge>
                    </td>
                    <td className="py-1 pr-2 text-right font-mono">
                      {r.ttft_ms !== null ? formatDuration(r.ttft_ms) : "—"}
                    </td>
                    <td className="py-1 pr-2 text-right font-mono">
                      {formatDuration(r.duration_ms)}
                    </td>
                    <td className="py-1 pr-2 text-right font-mono">
                      {r.tokens_in > 0 ? r.tokens_in.toLocaleString() : "—"}
                    </td>
                    <td className="py-1 pr-2 text-right font-mono">
                      {r.tokens_out > 0 ? r.tokens_out.toLocaleString() : "—"}
                    </td>
                    <td className="py-1 pr-2 text-right font-mono text-emerald-500">
                      {r.cached_tokens > 0
                        ? r.cached_tokens.toLocaleString()
                        : "—"}
                    </td>
                    <td className="py-1 pr-2 text-right font-mono text-cyan-500">
                      {r.cache_creation_tokens > 0
                        ? r.cache_creation_tokens.toLocaleString()
                        : "—"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </CardContent>
    </Card>
  )
}
