import { clsx, type ClassValue } from "clsx"
import { twMerge } from "tailwind-merge"

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}

export function formatNumber(
  n: number,
  maximumFractionDigits: number = 1,
  notation: "standard" | "scientific" | "engineering" | "compact" = "compact"
): string {
  return Intl.NumberFormat("en", {
    notation,
    maximumFractionDigits,
  }).format(n)
}

export function formatTime(ms: number): string {
  if (ms < 1000) return `${ms.toFixed(0)}ms`
  const s = ms / 1000
  if (s < 60) return `${s.toFixed(1)}s`
  const m = s / 60
  if (m < 60) return `${m.toFixed(1)}m`
  const h = m / 60
  return `${h.toFixed(1)}h`
}
