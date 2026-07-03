# *my*bro

A HTTP reverse proxy between OpenAI/Anthropic-compatible clients and the
[Umans AI](https://api.code.umans.ai/v1) upstream. It tracks usage, gates
concurrency, and ships with a dashboard.

<img width="1109" height="1047" alt="image" src="https://github.com/user-attachments/assets/45f93210-1dff-4469-b2c6-572c358da112" />


## Features

- OpenAI (`/v1/chat/completions`, `/v1/models`) and Anthropic (`/v1/messages`) pipelines
- Multi-key pool with round-robin + cooldown + per-conversation affinity
- Semaphore-based concurrency gating with FIFO queue + 503 on overflow
- Retry with escalating backoff + key rotation on 500/503
- Vision handoff for models that can't see images (text description via a vision model)
- Tool JSON-Schema normalization (`$ref`/`$defs` inlining, nullable simplification)
- SSE streaming passthrough with backpressure
- In-memory ring buffer + SQLite for request stats (long-term history)
- Time-series dashboard with sparklines, per-key usage, distribution cards
- OpenTelemetry OTLP export (traces + metrics)
- Static binary, embedded frontend, musl-friendly

## Quick start

## Configuration

You can configure mybro in multiple ways; pick whichever fits your setup.
CLI flags override env vars, which override `config.json`.

### 1. `config.json`

Drop a `.config/config.json` next to the binary (auto-created with defaults if
missing):

```json
{
  "LISTEN_ADDR": "127.0.0.1:8084",
  "UPSTREAM_BASE_URL": "https://api.code.umans.ai/v1",
  "REQUEST_TIMEOUT": "15m",
  "WEBSEARCH_PROVIDER": "none",
  "API_KEY": "sk-your-umans-key",
  "KEYS": [
    { "name": "Primary", "key": "sk-...", "session": "" }
  ],
  "OVERRIDE_CONCURRENCY": 0,
  "MAX_IMAGES": 9,
  "VISION_HANDOFF_ENABLED": false
}
```

Use this for persistent settings that survive restarts.

### 2. Environment variables

For containers or quick overrides, use the same keys:

```bash
UMANS_API_KEY="sk-your-key" \
LISTEN_ADDR="0.0.0.0:8084" \
OVERRIDE_CONCURRENCY=8 \
MAX_IMAGES=12 \
mybro
```

Supports: `LISTEN_ADDR`, `UPSTREAM_BASE_URL`, `REQUEST_TIMEOUT`,
`UMANS_API_KEY`, `API_KEYS`, `OVERRIDE_CONCURRENCY`, `MAX_IMAGES`,
`VISION_HANDOFF_ENABLED`, `VISION_HANDOFF_CACHE_ENABLED`,
`VISION_HANDOFF_CACHE_TTL`, `WEBSEARCH_PROVIDER`, `OTEL_EXPORTER_OTLP_ENDPOINT`.

### 3. CLI flags

For dev/test/scripting, see [CLI flags](#cli-flags) below. Most useful:

```bash
cargo run -- --mock-upstream 9001 --mock-delay-ms 300 --mock-concurrency 4
```

### 4. Dashboard

`POST /api/config` and `POST /api/keys` mutate config at runtime. Changes
take effect immediately for new requests and are persisted via debounced
write to `config.json`.

## CLI flags

```
--otel-endpoint <url>      OTLP HTTP collector endpoint
--otel-enabled             Enable OTel export
--otel-service-name <name> OTel service name (default: mybro)
--dev-proxy [url]          Proxy non-API routes to a Vite dev server
--mock-upstream <port>     Start a fake UMANS server on that port
--mock-delay-ms <ms>       Add artificial latency to mock responses
--mock-concurrency <n>     Mock's reported concurrency limit (default 8)
```

## Endpoints

| Route | Purpose |
|-------|---------|
| `/`, `/dashboard` | Embedded Vite/React dashboard |
| `/healthz` | Health + uptime + token state |
| `/v1/chat/completions` | OpenAI-compatible chat |
| `/v1/messages`, `/messages` | Anthropic-compatible messages |
| `/v1/models`, `/v1/models/info` | OpenAI-format model list + raw catalog |
| `/api/config` | GET/POST config (masked) |
| `/api/keys` | GET/POST key management |
| `/api/models` | Dashboard model view |
| `/api/stats` | Time-series stats (`?window=N&bucket=N&mode=buckets|summary|recent`) |
| `/api/stats/tokens` | Per-key usage breakdown |
| `/api/umans/usage` | Usage data from upstream |
| `/api/umans/concurrency` | Live concurrency + gate state |
| `/api/restart` | POST → graceful exit 42 (for `listenAddr` changes) |

## Docker

```bash
docker build -t mybro .
docker run -p 8084:8084 \
  -v $(pwd)/.config:/app/.config \
  -v $(pwd)/.data:/app/.data \
  -v $(pwd)/.logs:/app/.logs \
  mybro
```

## Development

```bash
# Backend (with embedded assets)
cargo run -- --mock-upstream 9001

# Frontend dev server (Vite proxies API calls to :8084)
cd uman-frontend && pnpm dev
```

## OpenTelemetry

Set `OTEL_EXPORTER_OTLP_ENDPOINT=http://your-collector:4318` and pass
`--otel-enabled`. Exports traces (one span per request) + metrics
(request count, latency, errors) via OTLP/HTTP.

## CI / Releases

- `.github/workflows/ci.yml` — check, clippy, test, build, docker on push/PR
- `.github/workflows/release.yml` — on `v*.*.*` tags, builds and pushes
  multi-arch image to `ghcr.io/<owner>/mybro` + creates draft GitHub release
