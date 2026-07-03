import { Card, CardContent } from "@/components/ui/card"

interface MetricCardProps {
  title: string
  value: string
  subtitle: string
  sparkline: number[]
  color?: "primary" | "amber" | "emerald" | "rose" | "cyan"
}

const colorClasses = {
  primary: "text-primary",
  amber: "text-amber-500",
  emerald: "text-emerald-500",
  rose: "text-rose-500",
  cyan: "text-cyan-500",
}

export function MetricCard({ title, value, subtitle, sparkline, color = "primary" }: MetricCardProps) {
  const max = Math.max(...sparkline, 1)
  const min = Math.min(...sparkline, 0)
  const range = max - min || 1
  const width = 100
  const height = 40
  const points = sparkline
    .map((v, i) => {
      const x = (i / Math.max(sparkline.length - 1, 1)) * width
      const y = height - ((v - min) / range) * height
      return `${x},${y}`
    })
    .join(" ")

  return (
    <Card className="relative overflow-hidden">
      <svg
        className="absolute inset-0 h-full w-full opacity-10"
        preserveAspectRatio="none"
        viewBox={`0 0 ${width} ${height}`}
      >
        <polyline
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          points={points}
          className={colorClasses[color]}
        />
      </svg>
      <CardContent className="relative z-10 p-4">
        <div className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
          {title}
        </div>
        <div className={`mt-1 text-2xl font-semibold ${colorClasses[color]}`}>{value}</div>
        <div className="mt-0.5 text-xs text-muted-foreground">{subtitle}</div>
      </CardContent>
    </Card>
  )
}
