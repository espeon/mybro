import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Badge } from "@/components/ui/badge"
import { useHealth } from "@/hooks/use-health"

function formatUptime(seconds: number): string {
  if (seconds < 60) return `${seconds}s`
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ${seconds % 60}s`
  const h = Math.floor(seconds / 3600)
  const m = Math.floor((seconds % 3600) / 60)
  return `${h}h ${m}m`
}

export function HealthCard() {
  const { health, loading, error } = useHealth()

  if (error && !health) {
    return (
      <Card>
        <CardHeader>
          <CardTitle>Health</CardTitle>
        </CardHeader>
        <CardContent>
          <p className="text-sm text-destructive">{error}</p>
        </CardContent>
      </Card>
    )
  }

  if (!health) {
    return (
      <Card>
        <CardHeader>
          <CardTitle>Health</CardTitle>
        </CardHeader>
        <CardContent>
          <p className="text-sm text-muted-foreground">
            {loading ? "Loading…" : "No data"}
          </p>
        </CardContent>
      </Card>
    )
  }

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <CardTitle>Health</CardTitle>
          <Badge variant={health.ok ? "default" : "destructive"}>
            {health.ok ? "OK" : "DOWN"}
          </Badge>
        </div>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="grid grid-cols-2 gap-2 text-sm">
          <div>
            <span className="text-muted-foreground">Runtime</span>
            <p className="font-mono">{health.runtime} {health.runtime_version}</p>
          </div>
          <div>
            <span className="text-muted-foreground">Port</span>
            <p className="font-mono">{health.port}</p>
          </div>
          <div>
            <span className="text-muted-foreground">Uptime</span>
            <p className="font-mono">{formatUptime(health.uptime_sec)}</p>
          </div>
          <div>
            <span className="text-muted-foreground">Models</span>
            <p className="font-mono">{health.models_count}</p>
          </div>
          <div>
            <span className="text-muted-foreground">API Key</span>
            <p>
              <Badge variant={health.api_key_valid ? "default" : "secondary"}>
                {health.api_key_valid ? "Valid" : "Invalid"}
              </Badge>
            </p>
          </div>
          <div>
            <span className="text-muted-foreground">Tokens</span>
            <p className="font-mono">
              {health.valid_tokens}/{health.total_tokens} valid
            </p>
          </div>
        </div>
        <div>
          <span className="text-sm text-muted-foreground">Started at</span>
          <p className="font-mono text-xs">{health.started_at}</p>
        </div>
        {health.visionHandoff.enabled && (
          <div className="border-t pt-2">
            <span className="text-sm text-muted-foreground">Vision Handoff</span>
            <div className="mt-1 flex gap-2 text-xs">
              <Badge variant="outline">enabled</Badge>
              {health.visionHandoff.cacheEnabled && (
                <Badge variant="outline">cache: {health.visionHandoff.cache.size}/{health.visionHandoff.cache.maxSize}</Badge>
              )}
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  )
}