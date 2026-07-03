import { useState } from "react"
import { Button } from "@/components/ui/button"

interface LoginPageProps {
  onSuccess: () => void
}

export function LoginPage({ onSuccess }: LoginPageProps) {
  const [key, setKey] = useState("")
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    setError(null)
    setLoading(true)
    try {
      const resp = await fetch("/api/validate", {
        headers: { "X-Api-Key": key },
      })
      if (resp.status === 401) {
        setError("Invalid API key")
        setLoading(false)
        return
      }
      if (!resp.ok) {
        setError(`Server error: ${resp.status}`)
        setLoading(false)
        return
      }
      const json = await resp.json().catch(() => ({}))
      if (json.valid === false) {
        setError("Invalid API key")
        setLoading(false)
        return
      }
      // valid:true — could be (a) auth disabled or (b) the typed key works.
      // In case (a) the server is unauthenticated, so we don't actually need a key —
      // clear any previously-stored key so subsequent requests don't send garbage.
      if (key.trim() === "") {
        localStorage.removeItem("umans_api_key")
      } else {
        localStorage.setItem("umans_api_key", key)
      }
      onSuccess()
    } catch (e) {
      setError(e instanceof Error ? e.message : "Connection failed")
    } finally {
      setLoading(false)
    }
  }

  // If auth is disabled on the server, allow submitting with an empty key
  const submitEmpty = () => {
    setKey("")
    setError(null)
    setLoading(true)
    localStorage.removeItem("umans_api_key")
    onSuccess()
  }

  return (
    <div className="min-h-svh bg-background flex items-center justify-center p-4">
      <div className="w-full max-w-sm space-y-6">
        <div className="text-center">
          <h1 className="text-2xl font-semibold tracking-tight">mybro</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Sign in with your dashboard API key
          </p>
        </div>

        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <label htmlFor="key" className="text-sm font-medium">
              API key
            </label>
            <input
              id="key"
              type="password"
              autoFocus
              autoComplete="current-password"
              placeholder="sk-..."
              value={key}
              onChange={(e) => setKey(e.target.value)}
              className="h-10 w-full rounded-md border border-input bg-transparent px-3 text-sm shadow-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            />
            <p className="text-xs text-muted-foreground">
              Set via <code className="font-mono">API_KEYS</code> in config.json.
              Leave empty in config to disable auth.
            </p>
          </div>

          {error && (
            <div className="rounded-md border border-destructive bg-destructive/10 px-3 py-2 text-sm text-destructive">
              {error}
            </div>
          )}

          <Button type="submit" disabled={loading} className="w-full">
            {loading ? "Signing in…" : key ? "Sign in" : "Continue"}
          </Button>
          <button
            type="button"
            onClick={submitEmpty}
            disabled={loading}
            className="text-xs text-muted-foreground hover:text-foreground underline-offset-2 hover:underline"
          >
            Continue without a key (auth disabled)
          </button>
        </form>
      </div>
    </div>
  )
}