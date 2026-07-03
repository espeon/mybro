import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Badge } from "@/components/ui/badge"
import type { StatsSummary } from "@/lib/api"

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`
  return n.toString()
}

export function CacheCard({ summary }: { summary: StatsSummary | null }) {
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

  const rate = summary.cache_hit_rate
  const pct = (rate * 100).toFixed(1)
  const hasCache = summary.cached_tokens > 0
  const variant = hasCache ? "default" : "secondary"

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <CardTitle>Prompt Cache</CardTitle>
          <Badge variant={variant}>
            {hasCache ? `${pct}% hits` : "no hits yet"}
          </Badge>
        </div>
      </CardHeader>
      <CardContent className="space-y-2 text-sm">
        <div className="grid grid-cols-2 gap-2">
          <div>
            <span className="text-muted-foreground">Hit rate</span>
            <p className="font-mono text-lg font-semibold">{pct}%</p>
          </div>
          <div>
            <span className="text-muted-foreground">Cached tokens</span>
            <p className="font-mono text-lg font-semibold">
              {formatTokens(summary.cached_tokens)}
            </p>
          </div>
        </div>
        <div className="grid grid-cols-2 gap-2 pt-2 border-t">
          <div>
            <span className="text-muted-foreground">Total in</span>
            <p className="font-mono">{formatTokens(summary.tokens_in)}</p>
          </div>
          <div>
            <span className="text-muted-foreground">Cache requests</span>
            <p className="font-mono">{summary.cached}</p>
          </div>
        </div>
        <p className="text-xs text-muted-foreground pt-2">
          Parsed from upstream usage block (OpenAI cached_tokens / Anthropic cache_read_input_tokens).
          Reflects the model's own prompt cache.
        </p>
      </CardContent>
    </Card>
  )
}