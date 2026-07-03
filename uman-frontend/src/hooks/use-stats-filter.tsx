import { createContext, useContext, useState, useCallback, useEffect, type ReactNode } from "react"
import { api } from "@/lib/api"

export const WINDOW_OPTIONS = [
  { label: "5m", value: 300 },
  { label: "15m", value: 900 },
  { label: "30m", value: 1800 },
  { label: "1h", value: 3600 },
  { label: "6h", value: 21600 },
  { label: "24h", value: 86400 },
]

interface StatsFilterState {
  window: number
  model: string
  models: string[]
  setWindow: (w: number) => void
  setModel: (m: string) => void
}

const Ctx = createContext<StatsFilterState | null>(null)

export function StatsFilterProvider({ children }: { children: ReactNode }) {
  const [window, setWindow] = useState(300)
  const [model, setModel] = useState("")
  const [models, setModels] = useState<string[]>([])

  // Fetch available model names whenever the window changes.
  // Uses a wider window (max of current window, 3600) so the dropdown doesn't
  // empty out when switching to a short window that has no data yet.
  const refreshModels = useCallback(async () => {
    try {
      const fetchWindow = Math.max(window, 3600)
      const resp = await api.getStatsModels(fetchWindow)
      setModels(resp.models || [])
    } catch {
      // ignore — non-critical
    }
  }, [window])

  useEffect(() => {
    refreshModels()
    const interval = setInterval(refreshModels, 30000)
    return () => clearInterval(interval)
  }, [refreshModels])

  // If the selected model is no longer in the list, clear it
  useEffect(() => {
    if (model && models.length > 0 && !models.includes(model)) {
      setModel("")
    }
  }, [model, models])

  return (
    <Ctx.Provider value={{ window, model, models, setWindow, setModel }}>
      {children}
    </Ctx.Provider>
  )
}

export function useStatsFilter() {
  const ctx = useContext(Ctx)
  if (!ctx) throw new Error("useStatsFilter must be used within StatsFilterProvider")
  return ctx
}
