import { useEffect, useState, useCallback } from "react"
import { api, type GateState } from "@/lib/api"

export function useGate() {
  const [gate, setGate] = useState<GateState | null>(null)
  const [error, setError] = useState<string | null>(null)

  const refresh = useCallback(async () => {
    try {
      const data = await api.getGate()
      setGate(data)
      setError(null)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }, [])

  useEffect(() => {
    refresh()
    const interval = setInterval(refresh, 1000)
    return () => clearInterval(interval)
  }, [refresh])

  return { gate, error, refresh }
}