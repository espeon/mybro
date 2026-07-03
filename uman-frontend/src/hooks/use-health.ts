import { useEffect, useState, useCallback } from "react"
import { api, type Healthz } from "@/lib/api"

export function useHealth() {
  const [health, setHealth] = useState<Healthz | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)

  const refresh = useCallback(async () => {
    try {
      setError(null)
      const data = await api.healthz()
      setHealth(data)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    refresh()
    const interval = setInterval(refresh, 15000)
    return () => clearInterval(interval)
  }, [refresh])

  return { health, error, loading, refresh }
}
