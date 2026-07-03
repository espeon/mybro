import { useEffect, useState, useCallback } from "react"
import { api, type RequestRecord } from "@/lib/api"
import { useStatsFilter } from "./use-stats-filter"

export function useRecentRequests(limit = 50, model?: string) {
  const { paused } = useStatsFilter()
  const [records, setRecords] = useState<RequestRecord[]>([])
  const [loading, setLoading] = useState(true)

  const refresh = useCallback(async () => {
    try {
      const data = await api.getStatsRecent(limit, model)
      setRecords(data.records)
    } catch {
      // ignore
    } finally {
      setLoading(false)
    }
  }, [limit, model])

  useEffect(() => {
    refresh()
    if (paused) return
    const interval = setInterval(refresh, 3000)
    return () => clearInterval(interval)
  }, [refresh, paused])

  return { records, loading, refresh }
}
