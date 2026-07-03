import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Badge } from "@/components/ui/badge"
import type { StatsSummary } from "@/lib/api"

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`
  return n.toString()
}

export function CacheCard({
  summary,
  error,
}: {
  summary: StatsSummary | null
  error: string | null
}) {
  if (error) {
    return (
      <Card>
        <CardHeader>
          <CardTitle>Prompt Cache</CardTitle>
        </CardHeader>
        <CardContent>
          <p className="text-sm text-destructive">{error}</p>
        </CardContent>
      </Card>
    )
  }
  if (!summary) {
    return (
      <Card>
        <CardHeader>
          <CardTitle>Prompt Cache</CardTitle>
        </CardHeader>
        <CardContent>
          <p className="text-sm text-muted-foreground">Loading…</p>
        </CardContent>
      </Card>
    )
  }

  const hitRate = summary.cache_hit_rate
  const hitPct = (hitRate * 100).toFixed(1)
  const hasHits = summary.cached_tokens > 0
  const hasWrites = summary.cache_creation_tokens > 0
  const hasAny = hasHits || hasWrites

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <CardTitle>Prompt Cache</CardTitle>
          <Badge variant={hasAny ? "default" : "secondary"}>
            {hasAny ? `${hitPct}% hits` : "no hits yet"}
          </Badge>
        </div>
      </CardHeader>
      <CardContent className="space-y-2 text-sm">
        <div className="grid grid-cols-2 gap-2">
          <div>
            <span className="text-muted-foreground">Hit rate</span>
            <p className="font-mono text-lg font-semibold">{hitPct}%</p>
          </div>
          <div>
            <span className="text-muted-foreground">Cache reqs</span>
            <p className="font-mono text-lg font-semibold">{summary.cached}</p>
          </div>
        </div>
        <div className="grid grid-cols-2 gap-2 border-t pt-2">
          <div>
            <span className="text-muted-foreground">Hits (read)</span>
            <p className="font-mono text-emerald-500">
              {formatTokens(summary.cached_tokens)}
            </p>
          </div>
          <div>
            <span className="text-muted-foreground">Writes (create)</span>
            <p className="font-mono text-cyan-500">
              {formatTokens(summary.cache_creation_tokens)}
            </p>
          </div>
        </div>
        <div className="grid grid-cols-2 gap-2 border-t pt-2">
          <div>
            <span className="text-muted-foreground">Total in</span>
            <p className="font-mono">{formatTokens(summary.tokens_in)}</p>
          </div>
          <div>
            <span className="text-muted-foreground">Total out</span>
            <p className="font-mono">{formatTokens(summary.tokens_out)}</p>
          </div>
        </div>
      </CardContent>
    </Card>
  )
}
