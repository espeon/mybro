import { useEffect, useState, useCallback } from "react"
import { api, type ModelsResponse } from "@/lib/api"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"

export function ModelsCard() {
  const [models, setModels] = useState<ModelsResponse | null>(null)
  const [error, setError] = useState<string | null>(null)

  const refresh = useCallback(async () => {
    try {
      setError(null)
      const data = await api.getModels()
      setModels(data)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }, [])

  useEffect(() => {
    refresh()
  }, [refresh])

  const toggleModel = async (id: string, currentlyDisabled: boolean) => {
    if (!models) return
    const newDisabled = currentlyDisabled
      ? models.disabled.filter((m) => m !== id)
      : [...models.disabled, id]
    try {
      await api.postConfig({ disabledModels: newDisabled })
      setModels({ ...models, disabled: newDisabled })
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <CardTitle>Models</CardTitle>
          <Button variant="outline" size="sm" onClick={refresh}>
            Refresh
          </Button>
        </div>
      </CardHeader>
      <CardContent>
        {error && <p className="text-sm text-destructive">{error}</p>}
        {!models && !error && <p className="text-sm text-muted-foreground">Loading...</p>}
        {models && (
          <div className="space-y-2">
            {models.models.map((model) => {
              const isDisabled = models.disabled.includes(model.id)
              return (
                <div
                  key={model.id}
                  className="flex items-center justify-between rounded-md border p-2"
                >
                  <div className="space-y-1">
                    <div className="flex items-center gap-2">
                      <span className="text-sm font-medium">{model.displayName}</span>
                      {model.reasoning && <Badge variant="outline">reasoning</Badge>}
                      {model.supportsTools && <Badge variant="outline">tools</Badge>}
                      {model.supportsVision !== "false" && (
                        <Badge variant="outline">{model.supportsVision}</Badge>
                      )}
                    </div>
                    <p className="font-mono text-xs text-muted-foreground">
                      {model.id} · ctx {model.contextWindow > 0 ? `${Math.round(model.contextWindow / 1000)}k` : "?"}
                    </p>
                  </div>
                  <Button
                    variant={isDisabled ? "secondary" : "default"}
                    size="sm"
                    onClick={() => toggleModel(model.id, isDisabled)}
                  >
                    {isDisabled ? "Enable" : "Disable"}
                  </Button>
                </div>
              )
            })}
            {models.models.length === 0 && (
              <p className="text-sm text-muted-foreground">No models available</p>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  )
}
