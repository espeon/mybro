const API_BASE = ""

function getAuthHeaders(): Record<string, string> {
  const key = localStorage.getItem("umans_api_key") || ""
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
  }
  if (key) {
    headers["X-Api-Key"] = key
  }
  return headers
}

async function fetchJSON<T>(path: string, options?: RequestInit): Promise<T> {
  const resp = await fetch(`${API_BASE}${path}`, {
    ...options,
    headers: {
      ...getAuthHeaders(),
      ...(options?.headers || {}),
    },
  })
  if (!resp.ok) {
    const text = await resp.text().catch(() => resp.statusText)
    throw new Error(`${resp.status}: ${text}`)
  }
  return resp.json()
}

// ── Types ────────────────────────────────────────────────────────────────────

export interface Healthz {
  ok: boolean
  started_at: string
  uptime_sec: number
  api_key_valid: boolean
  provider: string
  token_state: TokenState[]
  valid_tokens: number
  total_tokens: number
  models_count: number
  runtime: string
  runtime_version: string
  port: number
  visionHandoff: {
    enabled: boolean
    cacheEnabled: boolean
    cache: {
      size: number
      maxSize: number
      ttlMs: number
      hits: number
      misses: number
      evictions: number
    }
  }
}

export interface TokenState {
  name: string
  status: "active" | "cooldown" | "none"
  healthy: boolean
  remainingCooldown: number
  token: string
}

export interface SafeKey {
  name: string
  token_masked: string
  has_token: boolean
  has_session: boolean
}

export interface KeysResponse {
  keys: Array<{ name: string; key: string; session: string }>
  safe: SafeKey[]
}

export interface DashboardModel {
  id: string
  displayName: string
  reasoning: boolean
  variants?: Record<string, { thinking: { type: string; budget_tokens: number } }>
  supportsTools: boolean
  supportsVision: string
  contextWindow: number
}

export interface ModelsResponse {
  models: DashboardModel[]
  disabled: string[]
  displayNames: Record<string, string>
}

export interface UsageData {
  usage: {
    requests_in_window: number
    tokens_in: number
    tokens_out: number
    tokens_cached: number
  }
  window: { started_at: string }
  plan: { display_name: string }
  throttled: number
}

export interface ConcurrencyData {
  concurrent: number
  limit: number | null
  hard_cap: number | null
  user_id: string
  overridden: boolean
  active: number
  queued: number
}

// ── API calls ────────────────────────────────────────────────────────────────

export const api = {
  healthz: () => fetchJSON<Healthz>("/healthz"),
  getKeys: () => fetchJSON<KeysResponse>("/api/keys"),
  postKeys: (body: unknown) =>
    fetchJSON<{ success: boolean }>("/api/keys", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  getModels: () => fetchJSON<ModelsResponse>("/api/models"),
  getConfig: () => fetchJSON<Record<string, unknown>>("/api/config"),
  postConfig: (body: unknown) =>
    fetchJSON<Record<string, unknown>>("/api/config", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  getUsage: (fresh = true) =>
    fetchJSON<UsageData>(`/api/umans/usage${fresh ? "?fresh=1" : ""}`),
  getConcurrency: (fresh = true) =>
    fetchJSON<ConcurrencyData>(`/api/umans/concurrency${fresh ? "?fresh=1" : ""}`),
  validate: () => fetchJSON<{ valid: boolean }>("/api/validate"),
  restart: () =>
    fetchJSON<{ success: boolean; message: string }>("/api/restart", {
      method: "POST",
    }),
  // ── Stats (time-series) ──────────────────────────────────────────────────
  getStats: (window = 300, bucket = 10, mode = "buckets") =>
    fetchJSON<StatsResponse>(`/api/stats?window=${window}&bucket=${bucket}&mode=${mode}`),
  getStatsSummary: (window = 300) =>
    fetchJSON<StatsSummary>(`/api/stats?window=${window}&mode=summary`),
  getStatsRecent: (limit = 50) =>
    fetchJSON<{ records: RequestRecord[] }>(`/api/stats?mode=recent&limit=${limit}`),
}

// ── Stats types ─────────────────────────────────────────────────────────────

export interface StatsBucket {
  ts_ms: number
  count: number
  errors: number
  throttled: number
  avg_latency_ms: number
  p50_latency_ms: number
  p95_latency_ms: number
  max_latency_ms: number
  avg_ttft_ms: number
  p50_ttft_ms: number
  p95_ttft_ms: number
  tokens_in: number
  tokens_out: number
  cached: number
  cached_tokens: number
  cache_hit_rate: number
  by_model: Record<string, {
    count: number
    latency_sum_ms: number
    tokens_in: number
    tokens_out: number
  }>
}

export interface StatsSummary {
  count: number
  errors: number
  throttled: number
  cached: number
  cached_tokens: number
  cache_hit_rate: number
  tokens_in: number
  tokens_out: number
  avg_latency_ms: number
  avg_ttft_ms: number
}

export interface StatsResponse {
  buckets: StatsBucket[]
  summary: StatsSummary
  window_sec: number
  bucket_sec: number
}

export interface RequestRecord {
  ts_ms: number
  duration_ms: number
  status: number
  model: string
  pipeline: string
  key_name: string
  tokens_in: number
  tokens_out: number
  cached: boolean
  error: string | null
}
