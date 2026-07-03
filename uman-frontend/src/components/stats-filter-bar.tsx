import { useStatsFilter, WINDOW_OPTIONS } from "@/hooks/use-stats-filter"
import { Button } from "@/components/ui/button"
import { Select } from "@/components/ui/select"

export function StatsFilterBar() {
  const { window, setWindow, model, models, setModel } = useStatsFilter()

  return (
    <div className="flex flex-wrap items-center gap-2">
      <div className="flex flex-wrap items-center gap-1">
        {WINDOW_OPTIONS.map((opt) => (
          <Button
            key={opt.value}
            variant={window === opt.value ? "default" : "outline"}
            size="sm"
            className="h-7 px-2 text-xs"
            onClick={() => setWindow(opt.value)}
          >
            {opt.label}
          </Button>
        ))}
      </div>
      <Select
        value={model || "__all__"}
        onChange={(e) =>
          setModel(e.target.value === "__all__" ? "" : e.target.value)
        }
      >
        <option value="__all__">All models</option>
        {models.map((m) => (
          <option key={m} value={m}>
            {m}
          </option>
        ))}
      </Select>
    </div>
  )
}
