import { useEffect, useState, useCallback } from "react"
import { api, type KeysResponse } from "@/lib/api"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"

export function KeysCard() {
  const [keys, setKeys] = useState<KeysResponse | null>(null)
  const [error, setError] = useState<string | null>(null)

  const refresh = useCallback(async () => {
    try {
      setError(null)
      const data = await api.getKeys()
      setKeys(data)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }, [])

  useEffect(() => {
    refresh()
  }, [refresh])

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <CardTitle>API Keys</CardTitle>
          <Button variant="outline" size="sm" onClick={refresh}>
            Refresh
          </Button>
        </div>
      </CardHeader>
      <CardContent>
        {error && <p className="text-sm text-destructive">{error}</p>}
        {!keys && !error && <p className="text-sm text-muted-foreground">Loading...</p>}
        {keys && (
          <div className="space-y-2">
            {keys.safe.map((key, i) => (
              <div key={i} className="flex items-center justify-between rounded-md border p-2">
                <div className="space-y-1">
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-medium">{key.name}</span>
                    <Badge variant={key.has_token ? "default" : "secondary"}>
                      {key.has_token ? "has key" : "no key"}
                    </Badge>
                    {key.has_session && <Badge variant="outline">session</Badge>}
                  </div>
                  <p className="font-mono text-xs text-muted-foreground">{key.token_masked}</p>
                </div>
              </div>
            ))}
            {keys.safe.length === 0 && (
              <p className="text-sm text-muted-foreground">No keys configured</p>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  )
}
