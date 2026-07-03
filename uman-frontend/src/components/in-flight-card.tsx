import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Badge } from "@/components/ui/badge"

export function InFlightCard({
  active,
  queued,
  limit,
  hardCap,
  maxQueue,
  throttled,
}: {
  active: number
  queued: number
  limit: number | null
  hardCap: number | null
  maxQueue: number
  throttled: number
}) {
  const effectiveLimit = hardCap ?? limit ?? 4
  const fillPct = Math.min(100, (active / effectiveLimit) * 100)
  const isFull = active >= effectiveLimit

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <CardTitle>In-Flight</CardTitle>
          <Badge variant={isFull ? "destructive" : active > 0 ? "default" : "secondary"}>
            {active}/{effectiveLimit}
          </Badge>
        </div>
      </CardHeader>
      <CardContent className="space-y-3">
        {/* Progress bar */}
        <div className="space-y-1">
          <div className="h-2 w-full overflow-hidden rounded-full bg-muted">
            <div
              className={`h-full transition-all ${
                isFull
                  ? "bg-destructive"
                  : fillPct > 75
                    ? "bg-amber-500"
                    : "bg-primary"
              }`}
              style={{ width: `${fillPct}%` }}
            />
          </div>
          <div className="flex justify-between text-xs text-muted-foreground">
            <span>{fillPct.toFixed(0)}% capacity</span>
            {hardCap && <span>hard cap {hardCap}</span>}
          </div>
        </div>

        {/* Stats grid */}
        <div className="grid grid-cols-3 gap-2 pt-1 text-sm">
          <div>
            <span className="text-muted-foreground">Active</span>
            <p className="font-mono text-lg font-semibold">{active}</p>
          </div>
          <div>
            <span className="text-muted-foreground">Queued</span>
            <p className="font-mono text-lg font-semibold">{queued}</p>
          </div>
          <div>
            <span className="text-muted-foreground">Throttled</span>
            <p className="font-mono text-lg font-semibold text-destructive">
              {throttled}
            </p>
          </div>
        </div>
        <p className="text-xs text-muted-foreground">
          Queue capacity: {maxQueue} (HTTP 503 if full)
        </p>
      </CardContent>
    </Card>
  )
}