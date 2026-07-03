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

const STORAGE_KEY = "mybro_stats_filter"
const DEFAULT_WINDOW = 300

interface PersistedFilter {
  window: number
  model: string
}

function loadFilter(): PersistedFilter {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<PersistedFilter>
      return {
        window: typeof parsed.window === "number" ? parsed.window : DEFAULT_WINDOW,
        model: typeof parsed.model === "string" ? parsed.model : "",
      }
    }
  } catch {
    // ignore corrupt JSON
  }
  return { window: DEFAULT_WINDOW, model: "" }
}

function saveFilter(f: PersistedFilter) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(f))
  } catch {
    // ignore quota errors
  }
}

interface StatsFilterState {
  window: number
  model: string
  models: string[]
  paused: boolean
  setWindow: (w: number) => void
  setModel: (m: string) => void
  togglePaused: () => void
}

const Ctx = createContext<StatsFilterState | null>(null)

export function StatsFilterProvider({ children }: { children: ReactNode }) {
  const initial = loadFilter()
  const [window, setWindowState] = useState(initial.window)
  const [model, setModelState] = useState(initial.model)
  const [models, setModels] = useState<string[]>([])
  const [paused, setPaused] = useState(false)

  const setWindow = useCallback((w: number) => {
    setWindowState(w)
  }, [])

  const setModel = useCallback((m: string) => {
    setModelState(m)
  }, [])

  const togglePaused = useCallback(() => {
    setPaused((p) => !p)
  }, [])

  // Persist to localStorage whenever the filter changes
  useEffect(() => {
    saveFilter({ window, model })
  }, [window, model])

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
      setModelState("")
    }
  }, [model, models])

  return (
    <Ctx.Provider value={{ window, model, models, paused, setWindow, setModel, togglePaused }}>
      {children}
    </Ctx.Provider>
  )
}

export function useStatsFilter() {
  const ctx = useContext(Ctx)
  if (!ctx) throw new Error("useStatsFilter must be used within StatsFilterProvider")
  return ctx
}
