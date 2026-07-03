import { useEffect, useState, useCallback } from "react"
import { api, type RequestRecord } from "@/lib/api"

export function useRecentRequests(limit = 50, model?: string) {
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
    const interval = setInterval(refresh, 3000)
    return () => clearInterval(interval)
  }, [refresh])

  return { records, loading, refresh }
}
