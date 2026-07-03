import { useEffect, useState, useCallback } from "react"

export interface GateState {
  active: number
  queued: number
  throttled: number
  limit: number | null
  hard_cap: number | null
  overridden: boolean
  max_queue_size: number
}

export function useGate() {
  const [gate, setGate] = useState<GateState | null>(null)

  const refresh = useCallback(async () => {
    try {
      const resp = await fetch("/api/umans/gate")
      if (resp.ok) {
        const data = await resp.json()
        setGate(data)
      }
    } catch {
      // ignore
    }
  }, [])

  useEffect(() => {
    refresh()
    const interval = setInterval(refresh, 1000)
    return () => clearInterval(interval)
  }, [refresh])

  return { gate, refresh }
}