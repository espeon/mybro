import { useEffect, useState, useCallback } from "react"
import { api, type RequestRecord } from "@/lib/api"

export function useRecentRequests(limit = 50) {
  const [records, setRecords] = useState<RequestRecord[]>([])
  const [loading, setLoading] = useState(true)

  const refresh = useCallback(async () => {
    try {
      const data = await api.getStatsRecent(limit)
      setRecords(data.records)
    } catch {
      // ignore
    } finally {
      setLoading(false)
    }
  }, [limit])

  useEffect(() => {
    refresh()
    const interval = setInterval(refresh, 3000)
    return () => clearInterval(interval)
  }, [refresh])

  return { records, loading, refresh }
}