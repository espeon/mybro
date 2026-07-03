import { useEffect, useState, useCallback } from "react"
import { api, type TokenSummaryResp } from "@/lib/api"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Badge } from "@/components/ui/badge"
import { useStatsFilter } from "@/hooks/use-stats-filter"

export function TokenStatsCard() {
  const { window, model } = useStatsFilter()
  const [tokens, setTokens] = useState<TokenSummaryResp[]>([])
  const [error, setError] = useState<string | null>(null)

  const refresh = useCallback(async () => {
    try {
      setError(null)
      const json = await api.getTokenStats(window, model || undefined)
      setTokens(json.tokens || [])
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }, [window, model])

  useEffect(() => {
    refresh()
    const interval = setInterval(refresh, 15000)
    return () => clearInterval(interval)
  }, [refresh])

  const windowLabel =
    window >= 3600 ? `${window / 3600}h` : `${window / 60}m`

  return (
    <Card>
      <CardHeader>
        <CardTitle>Per-Key Usage</CardTitle>
      </CardHeader>
      <CardContent>
        {error && <p className="text-sm text-destructive">{error}</p>}
        <div className="space-y-2">
          {tokens.map((t) => (
            <div
              key={t.key_name}
              className="flex items-center justify-between rounded-md border p-2"
            >
              <div className="space-y-1">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-medium">
                    {t.key_name || "Unknown"}
                  </span>
                  {t.errors > 0 && (
                    <Badge variant="destructive">{t.errors} errors</Badge>
                  )}
                </div>
                <p className="font-mono text-xs text-muted-foreground">
                  {t.count} reqs · {t.avg_latency_ms.toFixed(0)}ms avg
                </p>
              </div>
              <div className="text-right">
                <div className="font-mono text-xs">
                  {(t.tokens_in + t.tokens_out).toLocaleString()} tok
                </div>
              </div>
            </div>
          ))}
          {tokens.length === 0 && (
            <p className="text-sm text-muted-foreground">
              No data in last {windowLabel}
            </p>
          )}
        </div>
      </CardContent>
    </Card>
  )
}
