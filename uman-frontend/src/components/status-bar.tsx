import type { GateState } from "@/lib/api"
import { useHealth } from "@/hooks/use-health"

export function StatusBar({
  apiKey,
  gate,
}: {
  apiKey: string
  gate: GateState | null
}) {
  const { health } = useHealth()

  // Dot color reflects actual health, not just "signed in"
  const dotColor = !health
    ? "bg-muted-foreground" // still loading
    : health.ok
      ? "bg-emerald-500"
      : "bg-destructive"

  // If auth is disabled, the key is empty — show "auth disabled" instead of a weird suffix
  const authLabel = apiKey
    ? `Signed in as …${apiKey.slice(-6)}`
    : "auth disabled"

  return (
    <div className="flex items-center justify-between rounded-md border bg-card px-3 py-2 text-xs text-muted-foreground">
      <div className="flex items-center gap-2">
        <span
          className={`inline-block h-2 w-2 rounded-full ${dotColor}`}
          title={health?.ok ? "healthy" : "unhealthy"}
        />
        <span>{authLabel}</span>
      </div>
      {gate && (
        <div className="flex items-center gap-2">
          <span>{gate.active} in-flight</span>
          {gate.queued > 0 && (
            <span className="text-amber-500">{gate.queued} queued</span>
          )}
        </div>
      )}
    </div>
  )
}