import { useEffect, useState, useCallback } from "react"
import { api, type StatsSummary } from "@/lib/api"

export function useStatsSummary(window = 3600) {
  const [summary, setSummary] = useState<StatsSummary | null>(null)
  const [loading, setLoading] = useState(true)

  const refresh = useCallback(async () => {
    try {
      const data = await api.getStatsSummary(window)
      setSummary(data)
    } catch {
      // ignore
    } finally {
      setLoading(false)
    }
  }, [window])

  useEffect(() => {
    refresh()
    const interval = setInterval(refresh, 5000)
    return () => clearInterval(interval)
  }, [refresh])

  return { summary, loading, refresh }
}