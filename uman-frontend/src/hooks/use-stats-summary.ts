import { useEffect, useState, useCallback } from "react"
import { api, type StatsSummary } from "@/lib/api"
import { useStatsFilter } from "./use-stats-filter"

export function useStatsSummary(window = 3600, model?: string) {
  const { paused } = useStatsFilter()
  const [summary, setSummary] = useState<StatsSummary | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const refresh = useCallback(async () => {
    try {
      const data = await api.getStatsSummary(window, model)
      setSummary(data)
      setError(null)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setLoading(false)
    }
  }, [window, model])

  useEffect(() => {
    refresh()
    if (paused) return
    const interval = setInterval(refresh, 5000)
    return () => clearInterval(interval)
  }, [refresh, paused])

  return { summary, loading, error, refresh }
}
