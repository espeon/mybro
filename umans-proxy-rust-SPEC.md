# umans-proxy-rs — Technical Specification

> **Source**: SPEC.md from `umans-dash-go` (itself derived from a 3,326-line `proxy.js`).
> **Target**: Rust binary, tokio-based, optimized for throughput and low overhead.
> **Scope**: Full proxy + operator dashboard. Removed relative to the Go spec:
> opencode config discovery/setup and models.dev integration (both existed only to
> generate opencode configs). **Kept**: the dashboard, its `/api/*` surface, the
> wallpaper proxies, and the restart API.

---

## 1. Overview

A local HTTP reverse proxy between OpenAI/Anthropic-compatible clients and the
UMANS AI upstream (`https://api.code.umans.ai/v1`). Provides:

- Multi-key pool with round-robin, cooldown, and per-conversation key affinity
- Concurrency gating against the upstream's session limit, with a bounded FIFO queue
- Retry with escalating backoff and key rotation on 500/503/network errors
- Vision handoff: image → text description for models that can't see images (with LRU cache)
- Tool JSON-Schema normalization (`$ref`/`$defs` inlining, nullable simplification)
- Payload normalization (reasoning-field stripping, thinking-field casing, image capping)
- Streaming SSE passthrough with backpressure and disconnect propagation
- `/healthz` and OpenAI-format `/v1/models` with pricing

### Design principles

- **Async, zero-copy where it matters**: request bodies are read once into `Bytes`;
  streaming responses are piped as `Bytes` frames without buffering or re-encoding.
- **Parse once, mutate in place**: the JSON payload is deserialized into a single
  `serde_json::Value`, transformed through the whole pipeline, and serialized exactly
  once before the upstream call. No intermediate string round-trips.
- **Lock discipline**: no lock is held across `.await`. Shared state uses
  `arc_swap::ArcSwap` for read-mostly data (config, catalog) and `parking_lot::Mutex`
  for short critical sections (key pool, LRU, conversation map).
- **127.0.0.1 only**. Graceful shutdown on SIGINT/SIGTERM with in-flight drain.
- **Single static binary**: `rustls` (no OpenSSL), musl-friendly.

### Crate set

| Crate | Use |
|---|---|
| `tokio` (full) | Runtime, signals, timers, semaphore |
| `hyper` 1.x + `hyper-util` (or `axum` for routing sugar) | HTTP server + upstream client |
| `hyper-rustls` / `rustls` | TLS to upstream |
| `serde`, `serde_json` | Config + payload handling |
| `bytes` | Zero-copy body frames |
| `arc-swap` | Lock-free config/catalog snapshots |
| `parking_lot` | Fast non-async mutexes |
| `sha2`, `md-5` | Handoff cache key, conversation fingerprint |
| `tracing` + `tracing-subscriber` | Logging |
| `http-body-util` | Body plumbing |

`axum` is acceptable and recommended for the router; the hot path
(`/v1/chat/completions`, `/v1/messages`) must still stream via `Body::from_stream`
without buffering.

---

## 2. Configuration

### 2.1 File

Path: `.config/config.json` relative to the working directory.

```json
{
  "LISTEN_ADDR": "127.0.0.1:8084",
  "UPSTREAM_BASE_URL": "https://api.code.umans.ai/v1",
  "REQUEST_TIMEOUT": "15m",
  "API_KEY": "sk-...",
  "API_KEYS": ["proxy-key-1"],
  "KEYS": [
    { "name": "Default", "key": "sk-...", "session": "" }
  ],
  "ENABLED_MODELS": [],
  "MODEL_DISPLAY_NAMES": {},
  "OVERRIDE_CONCURRENCY": 0,
  "MAX_IMAGES": 9,
  "DISABLED_MODELS": [],
  "VISION_HANDOFF_ENABLED": false,
  "VISION_HANDOFF_MODEL": "umans-coder",
  "VISION_HANDOFF_PROMPT": "",
  "VISION_HANDOFF_CACHE_ENABLED": false,
  "VISION_HANDOFF_CACHE_TTL": "24h"
}
```

Deserialize with `#[serde(default)]` per field; unknown keys ignored
(`deny_unknown_fields` NOT set — forward compatibility with the old config file).
The parser must tolerate trailing commas is NOT required; standard JSON only.

### 2.2 Defaults

| Key | Default |
|---|---|
| `LISTEN_ADDR` | `127.0.0.1:8084` |
| `UPSTREAM_BASE_URL` | `https://api.code.umans.ai/v1` |
| `REQUEST_TIMEOUT` | `15m` |
| `OVERRIDE_CONCURRENCY` | `0` (0 = use API limit) |
| `MAX_IMAGES` | `9` |
| `VISION_HANDOFF_ENABLED` | `false` |
| `VISION_HANDOFF_MODEL` | `umans-coder` |
| `VISION_HANDOFF_PROMPT` | `""` (built-in default used) |
| `VISION_HANDOFF_CACHE_ENABLED` | `false` |
| `VISION_HANDOFF_CACHE_TTL` | `24h` |

### 2.3 Env overrides (applied after file parse)

| Env var | Config key | Notes |
|---|---|---|
| `LISTEN_ADDR` | `LISTEN_ADDR` | |
| `UPSTREAM_BASE_URL` | `UPSTREAM_BASE_URL` | |
| `REQUEST_TIMEOUT` | `REQUEST_TIMEOUT` | |
| `UMANS_API_KEY` | `API_KEY` | |
| `API_KEYS` | `API_KEYS` | comma-separated |
| `OVERRIDE_CONCURRENCY` | `OVERRIDE_CONCURRENCY` | parsed as `u32` |
| `MAX_IMAGES` | `MAX_IMAGES` | parsed as `usize` |
| `VISION_HANDOFF_ENABLED` | `VISION_HANDOFF_ENABLED` | `"false"` disables |
| `VISION_HANDOFF_CACHE_ENABLED` | `VISION_HANDOFF_CACHE_ENABLED` | `"false"` disables |
| `VISION_HANDOFF_CACHE_TTL` | `VISION_HANDOFF_CACHE_TTL` | |

### 2.4 Validation

`RequestTimeout` must be `> 0` after parsing; otherwise exit with a fatal error.

### 2.5 `parse_duration(&str) -> Duration`

- Regex-free hand parse: `^(\d+)(h|m|s)$` semantics.
- `h`/`m`/`s` → hours/minutes/seconds. Bare digits → **milliseconds**.
- Invalid/empty → `Duration::ZERO`.

### 2.6 `mask_token(&str) -> String`

First 10 chars + `...` + last 4 chars; if too short, `***`.

### 2.7 Runtime representation

```rust
pub struct Config { /* parsed, typed fields; includes wallpaper_source */ }
pub struct ConfigStore(ArcSwap<Config>);   // held in AppState
```

Config is **runtime-mutable** — `POST /api/config` and `POST /api/keys` edit it
live. Reads stay lock-free: every request path does `store.load()` once and works
from that `Arc<Config>` snapshot for the duration of the request (so a mid-flight
edit never tears a single request). Writes build a new `Config`, `store()` it, and
enqueue a persistence job. Because readers hold a snapshot `Arc`, there is no
reader/writer lock at all.

`wallpaper_source` (`none` | `bing` | `wallhaven`, default `bing`) returns as a
config field, since the dashboard exposes it.

### 2.8 Persistence — `debounced_save()`

A single background task owns disk writes. Mutating endpoints send a "dirty" ping
over a `tokio::sync::Notify` (or a `watch`); the task coalesces pings on a 500 ms
debounce and writes the current snapshot to `.config/config.json` atomically
(write temp + rename). This mirrors the Go `debouncedSaveConfig` without holding
any lock across the write. Keys are persisted in the `KEYS` array; env-var-derived
values are not written back.

---

## 3. Key Pool

Round-robin multi-key pool with cooldown.

### 3.1 Structure

```rust
pub struct KeyEntry {
    pub key: String,
    pub name: String,
    pub healthy: bool,
    pub last_error: Option<Instant>,
    pub cooldown: Duration,        // default 30s
}

pub struct KeyPool {
    inner: parking_lot::Mutex<PoolInner>,
}
struct PoolInner {
    entries: Vec<KeyEntry>,
    cursor: usize,
}
```

A `KeySlot` handed to callers is `{ index: usize, key: Arc<str>, name: Arc<str> }` —
a cheap clone that does not borrow the pool (no lock held during the upstream call).

### 3.2 `acquire(preferred: Option<usize>) -> Option<KeySlot>`

- If `preferred` is a valid index and that entry is healthy **or** past cooldown:
  mark healthy, return it.
- Else round-robin from `cursor`, returning the first healthy-or-cooldown-expired
  entry; mark it healthy, advance cursor.
- `None` if no usable keys. Entire operation under the mutex; O(n), n is tiny.

### 3.3 `mark_unhealthy(index, status: u16)`

`healthy = false`, `last_error = now`, cooldown by status:
- `>= 503` → 60 s
- `>= 502` → 30 s
- otherwise → 10 s

### 3.4 `mark_healthy(index)` — `healthy = true`, `last_error = None`.

### 3.5 `healthy_count()` — count of entries healthy or past cooldown.

### 3.6 `total()` — number of entries.

### 3.7 `state() -> Vec<KeyState>` (consumed by `/healthz`)

```rust
#[derive(Serialize)]
pub struct KeyState {
    name: String,
    status: &'static str,      // "active" | "cooldown" | "none"
    healthy: bool,
    #[serde(rename = "remainingCooldown")]
    remaining_cooldown_ms: u64,
    token: String,             // masked
}
```

`"none"` if key empty; `"active"` if healthy or cooldown expired; `"cooldown"` otherwise.

### 3.8 `rebuild(entries: Vec<KeyEntry>)` (consumed by `/api/keys`)

Replace the pool's entries in one locked swap and reset `cursor = 0`. In-flight
requests already hold a cloned `KeySlot` (§3.1), so they finish against their old
key unaffected; only subsequent `acquire()` calls see the new set. This is why the
slot is a detached clone rather than a borrow — live key edits become a non-event
for running requests.

---

## 4. Image Handoff Cache

LRU + TTL cache of image descriptions, keyed by SHA-256 of the image data URI.
Default 50 entries, 24 h TTL.

### 4.1 Structure

Hand-rolled intrusive LRU (or the `lru` crate) wrapped in `parking_lot::Mutex`:

```rust
pub struct HandoffCache {
    inner: Mutex<lru::LruCache<[u8; 32], CacheEntry>>,
    ttl: AtomicU64,          // ms, adjustable
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
}
struct CacheEntry { desc: Arc<str>, inserted: Instant }
```

Key is the raw 32-byte SHA-256 digest (not hex) — half the memory, no allocation.

### 4.2 Operations

- `get(&digest) -> Option<Arc<str>>`: hit if present and not expired (moves to
  front); expired entries are removed and counted as misses.
- `set(digest, desc)`: insert at front; evict LRU on capacity (count eviction).
- `stats() -> HandoffCacheStats`: `{size, maxSize, ttlMs, hits, misses, evictions}`.
- `resize(max, ttl)`.

### 4.3 Behavior

Only consulted when `VISION_HANDOFF_CACHE_ENABLED`. Checked before the handoff
upstream call; populated on success. Stats exposed at `/healthz` under
`visionHandoff.cache`.

---

## 5. Concurrency Gating & Queue

The Go spec's manual `activeRequests` counter + `Vec` queue maps to a cleaner and
faster Rust primitive: a **dynamically-resizable semaphore with a bounded waiter
count**.

### 5.1 Structure

```rust
pub struct Gate {
    sem: Arc<tokio::sync::Semaphore>,   // permits = effective gate limit
    granted: AtomicUsize,               // permits currently issued to sem
    active: AtomicUsize,                // in-flight proxied requests
    queued: AtomicUsize,                // tasks waiting on the semaphore
    throttled: AtomicU64,               // queue-full 503 count
}
pub const MAX_QUEUE_SIZE: usize = 256;
```

### 5.2 Admission

```text
gate_limit = effective.hard_cap.or(effective.limit)   // None => ungated
if gate_limit is None: run immediately (still count active).
else:
    if queued >= MAX_QUEUE_SIZE and no permit immediately available:
        throttled += 1; respond 503 {error: {message, type: "queue_full"}}
    else:
        queued += 1; permit = sem.acquire().await; queued -= 1
        run with permit held (RAII: dropping the permit releases it)
```

- FIFO fairness comes from tokio's semaphore (queued waiters are FIFO).
- No explicit `processQueue()` dispatcher is needed: permit drop wakes the next
  waiter. This removes an entire class of requeue bugs from the JS/Go designs.
- **Resizing**: when `get_effective_concurrency()` changes the limit, reconcile:
  `add_permits(new - granted)` if larger; if smaller, spawn a task that acquires
  and `forget()`s `granted - new` permits (shrink takes effect as requests finish).
- Both pipelines (OpenAI, Anthropic) go through the same gate.

### 5.3 Counters

`active` incremented when a request begins executing, decremented on completion
(use an RAII guard so panics/cancellations still decrement). `queued` and
`throttled` exposed via `/healthz`-adjacent internals and used in logs.

---

## 6. Usage Tracking & Effective Concurrency

### 6.1 `fetch_usage(fresh: bool) -> Option<Arc<UsageData>>`

- `GET {upstream}/usage`, `Authorization: Bearer <active key>`, 10 s timeout.
- 5-minute cache (`ArcSwap<Option<CachedUsage>>`); `fresh` bypasses.
- Non-OK → return stale cache if any, else `None`.
- **Throttle window reset**: if `window.started_at` differs from the stored window
  start, reset `throttled` to 0 and store the new start. HTTP + JSON parse happen
  with no lock held; only the compare-and-store is synchronized.
- Concurrent fetch dedup: a `tokio::sync::Mutex<()>` fetch guard or a
  `watch`-based single-flight so N callers trigger one upstream request.

### 6.2 `fetch_concurrency(fresh: bool)`

From usage data extract `{concurrent: usage.concurrent_sessions ?? 0,
limit: limits.concurrency.limit, hard_cap: limits.concurrency.hard_cap, user_id}`.
Store in `ArcSwap<LastConcurrency>`; invalidate the effective-concurrency cache
and reconcile the Gate (§5.2 resizing).

### 6.3 `get_effective_concurrency() -> Effective`

```text
if override > 0:
    hard_cap = api_hard_cap.map(|c| min(override, c)).unwrap_or(override)
    overridden = true
else: pass through API values, overridden = false
```
Cached in an `ArcSwap`; recomputed on concurrency refresh.

---

## 7. Upstream Client

### 7.1 Structure

One shared `hyper_util::client::legacy::Client` (or `reqwest::Client`) with:

- Keep-alive on, pool max idle per host **128**, idle timeout **60 s**
- rustls, HTTP/1.1 + HTTP/2 ALPN
- Per-request timeout via `tokio::time::timeout` wrapping only the
  **response-header phase** for streaming calls (a 15-minute stream must not be
  killed by a connect-timeout); total-duration timeout = `config.request_timeout`.

```rust
pub struct Upstream {
    base: Uri,
    client: HyperClient,
    timeout: Duration,
}
```

The API key is **not** stored on the client; it's passed per call from the
acquired `KeySlot` (this fixes the Go design's global-active-key mutation, which
was a race between concurrent requests on different keys).

### 7.2 Methods

- `get_user_info(key) -> Result<serde_json::Value>` — `GET {base}/models/info`,
  bearer auth, 10 s timeout.
- `chat_completions(key, body: Bytes, stream: bool) -> Result<Response<Incoming>>`
  — `POST {base}/chat/completions`; `Accept: text/event-stream` if `stream` else
  `application/json`; `Content-Type: application/json`.
- `messages(key, body: Bytes, stream: bool) -> Result<Response<Incoming>>` —
  `POST {base}/messages`, same headers.

All bodies are `Bytes` (single serialization of the mutated payload; retries reuse
the same `Bytes` — cheap refcount clone, no re-serialize).

---

## 8. Model Catalog

### 8.1 Fetch

`GET {base}/models/info` (optional bearer), 15 s timeout → `serde_json::Map` of
model id → info object.

### 8.2 Cached accessor

- TTL 5 min. Single-flight dedup of concurrent fetches (`tokio::sync::OnceCell`
  per generation, or a `Mutex<Option<JoinHandle>>` pattern).
- Failure → serve stale snapshot if present.

### 8.3 Snapshot representation

```rust
pub struct Catalog {
    pub info: HashMap<String, serde_json::Value>,       // model id → full info
    pub display: HashMap<String, String>,               // id → display name ("Umans " prefix stripped, case-insensitive)
    pub ordered_ids: Vec<String>,                       // precomputed
}
pub static CATALOG: ArcSwap<Arc<Catalog>>;
```

`applyCatalogData` builds a **new immutable snapshot** and `store()`s it — this
replaces the Go spec's `catalogMu` RWMutex entirely. `ordered_ids` (sorted by
lowercase display name, then id) is computed at snapshot build time, so the
Go-side "ordered ids cache invalidation" logic disappears.

### 8.4 Derived accessors

- `effective_models()`: `ordered_ids` minus `config.disabled_models`
  (fallback to `config.enabled_models` if the catalog is empty).
- `all_catalog_models()`: `ordered_ids` without the disabled filter (same fallback).

### 8.5 `fetch_upstream_models()` — pricing only, optional

`GET {base}/models`; 5-min cached; 10 s timeout; returns the `data` array. The
**only** field consumed is `pricing {input, output}` per id — confirmed against the
live api, `/models` carries no capabilities/reasoning/display_name, and its
`context_length` merely duplicates `capabilities.context_window` from `/models/info`.

Because of that, this is a **pricing-enrichment call, not a catalog source**: it is
fetched lazily inside the `/v1/models` handler (§20) and never on the proxy hot
path. If pricing in `/v1/models` isn't needed, this call — and the `pricing` block
in §20 — can be dropped outright, leaving `/models/info` as the single upstream
model endpoint. Gate it behind a `PRICING_ENABLED` flag (default on) so it's a
one-line toggle rather than a code change.

### 8.6 `validate_api_key() -> bool`

`get_user_info(active key)`; on success cache user info (5-min TTL) and apply as
a catalog snapshot; on failure log and return `false`.

### 8.7 Derived model capabilities (replaces models.dev)

Everything the dashboard and `/api/models` need is already in each umans catalog
entry's `capabilities` object — no external enrichment. Read directly from the
snapshot:

- `supports_vision`: `"true"` | `"false"` | `"via-handoff"` (drives §9.1).
- `supports_tools`: bool.
- `context_window`: number (drives `context_length` / `max_output_tokens`, §20).
- `reasoning`: `{ supported: bool, can_disable: bool, levels?: [..] }`.
- top-level `display_name`, `recommended_max_tokens`, `max_completion_tokens`.

**`reasoning_mode(caps) -> bool`**: `true` if `reasoning.supported == true` or
`reasoning.levels` is non-empty; else `false`. (The Go `resolveReasoningMode`
preferred a models.dev `reasoning_options` field, then fell back to exactly this.
Dropping models.dev drops only that fallback preference — the umans catalog is the
real source.)

**`reasoning_variants(caps) -> Option<Map>`**: `None` unless
`reasoning.supported == true` **and** `reasoning.can_disable == true`. Otherwise,
for each `level` in `reasoning.levels` except `"none"` that has an entry in
`REASONING_LEVEL_BUDGETS` (§23), emit:
```json
{ "thinking": { "type": "enabled", "budget_tokens": <budget> } }
```
Return the level→variant map, or `None` if no level yielded a budget. This is a
pure function of the umans caps plus the hardcoded budget table — byte-for-byte the
Go `buildReasoningVariants`, minus the models.dev plumbing.

> **Confirmed against the live `/models/info` (2026-07):** `capabilities.reasoning`
> is `{ supported: bool, can_disable: bool, levels: [..], default_level: str|null }`
> — `levels` and `can_disable` are always present, so the derivation is safe. Note
> that in the current catalog every model reports `supported: true, can_disable:
> false, levels: []`, which means `reasoning_variants` returns `None` for all of
> them today (variants require `can_disable == true`), and these models instead hit
> the auto-think path (§11.7: `supported && !can_disable` → `thinking = adaptive`).
> The two are mutually exclusive by design: a model whose reasoning can't be turned
> off is always-on, so there are no per-level variants to choose. The variant logic
> stays in for the day a model exposes `can_disable: true` with a non-empty `levels`.

---

## 9. Vision Handoff

Models with `capabilities.supports_vision == "via-handoff"` can't see images. The
proxy extracts images, describes each via a vision-capable model, and substitutes
text.

### 9.1 `needs_vision_handoff(resolved: &str) -> bool`

`false` unless `config.vision_handoff_enabled`; then true iff the catalog entry's
`capabilities.supports_vision` is the string `"via-handoff"`.

### 9.2 `resolve_model_id(requested: &str) -> String`

1. Starts with `umans-` → as-is.
2. `umans-{requested}` in effective models → prefixed.
3. Direct match in effective models → as-is.
4. Fallback → original.

### 9.3 `collect_image_parts(payload: &Value) -> Vec<ImageRef>`

Walk `payload.system` (if array) and every message's `content` array; recurse into
nested `content` arrays (tool_result blocks). Collect:

- OpenAI: `{"type":"image_url","image_url":{"url":u}}` → data URI = `u`.
- Anthropic `{"type":"image","source":{...}}`:
  - `source.type=="base64"` → `data:{media_type};base64,{data}`
  - `source.type=="url"` → the URL.

`ImageRef` stores a **JSON Pointer path** (e.g. `/messages/3/content/1`) instead of
raw pointers, so replacement is done safely on the same mutable `Value` after the
async analysis completes.

### 9.4 `analyze_image(data_uri, slot) -> String`

Non-streaming `chat_completions` to `config.vision_handoff_model` with the system
prompt (`config.vision_handoff_prompt` or the built-in default below) and a user
message of text + `image_url`. On success extract content (string, or concatenated
`text` parts). On any failure return `"[Image analysis failed: ...]"` — never abort
the parent request.

Cache lookup/store per §4 when enabled (key = SHA-256 of data URI).

### 9.5 `perform_vision_handoff(payload: &mut Value, resolved: &str) -> usize`

1. Skip unless `needs_vision_handoff`; collect parts; skip if none.
2. Analyze **all images concurrently** — `futures::future::join_all` over
   `analyze_image` futures (bounded by the handoff calls being few; no extra gate).
3. Replace each part in place with:
   ```json
   { "type": "text",
     "text": "[Image content — analyzed by vision module, shown as text because the active model cannot see images:]\n{desc}" }
   ```
   Multi-image label: `[Image {i+1} content — ...]`.
4. Return count.

### 9.6 SSE keepalive during handoff

For streaming requests that will hand off: before analysis, send response headers
(`Content-Type: text/event-stream`, `Cache-Control: no-cache`) plus the comment
frame `: keepalive — analyzing image via vision handoff\n\n` through the streaming
body channel. Once headers are flushed, any later error must be emitted as an SSE
`data:` event, not a JSON response (track `headers_sent` in the response writer).

### 9.7 Default handoff prompt

```
You are an image captioning module. Your output is fed verbatim into another model as the sole visual content of the image — it cannot see the image itself, only your text.

Produce a factual, third-person description of the image contents. Do NOT use first person ("I see..."). Do NOT address the reader. Do NOT speculate about what the user wants.

Cover:
- Type of image (screenshot, photograph, diagram, UI, log, etc.) and overall layout
- All visible elements (objects, UI widgets, people, regions) and their spatial arrangement
- Exact transcription of any visible text, code, or labels (use quotes)
- Salient technical details (file paths, error messages, colors, dimensions, filenames)

Write as a single coherent description, not a bulleted list. Be thorough but concise.
```

---

## 10. Tool Schema Normalization

Same algorithm as the source; operates on `serde_json::Value` in place.

### 10.1 `normalize_tool_schemas(tools: &mut Value)`

Fast-path guard first: serialize-free scan — only run if any tool's
`function.parameters` object contains a `$defs`, `definitions`, or `$ref` key at
any depth (a cheap recursive key scan; bail out on first hit). For each tool,
extract local definitions (`definitions` + `$defs`) and run `normalize_schema`
with `max_depth = 12`.

### 10.2 `normalize_schema(node, defs, depth)`

1. `depth == 0` → return node cloned as-is.
2. Merge node-local `definitions`/`$defs` into `defs` (a `HashMap<&str, &Value>` of
   borrowed refs; clone only when a `$ref` actually resolves).
3. If the node is exactly `{"$ref": "#/definitions/x" | "#/$defs/x"}` (one key) and
   the target exists → recurse on a clone of the definition.
4. Otherwise recurse into every value, skipping keys `definitions`, `$defs`,
   `nullable`.
5. Post-passes on the object:
   - `simplify_nullable_combinator` for `anyOf`, `oneOf`
   - `normalize_type_field`
   - `normalize_enum_field`
   - remove `const: null`

### 10.3 `simplify_nullable_combinator(schema, key)`

Filter null-schemas (`{"type":"null"}`, `{"const":null}`, `{"enum":[null]}`) out of
the array. Empty → remove key. One left → inline (merge its keys into the parent,
remove key). Else keep the filtered array.

### 10.4 `normalize_type_field`

If `type` is an array: drop `"null"` and empty strings; empty → remove `type`;
else `type = first remaining`.

### 10.5 `normalize_enum_field`

If `enum` is an array: drop `null`s; dedupe by `(discriminant, canonical JSON)`
key; empty → remove; else keep.

---

## 11. Payload Normalization

### 11.1 `strip_reasoning_content(payload)`

For each `role == "assistant"` message remove `reasoning_content` and
`reasoningContent`.

### 11.2 `normalize_thinking_payload(payload)`

If `payload.thinking` is an object with `budgetTokens` and without `budget_tokens`:
rename `budgetTokens` → `budget_tokens`. (Undoes `@ai-sdk/openai-compatible`
camelCasing.)

### 11.3 `limit_images_in_messages(payload, max_images)`

No-op if `max_images == 0`. Walk `system` array + all message `content` arrays
collecting image parts (`image_url` / `image`) with positions (`-1` for system).
If count ≤ max → no-op. Otherwise replace the **oldest** `count - max` parts with
`{"type":"text","text":"(Image previously shared)"}` (order = ascending message
index; stable collection order suffices).

### 11.4 `fingerprint_payload(payload) -> String`

MD5 of the **first** user message's text (per `msg_text`), hex-truncated to 12
chars. Empty string if none. (MD5 is fine — non-cryptographic bucketing only.)

### 11.5 `msg_text(m) -> &str`

String content → itself; array content → `text` of the first `{"type":"text"}`
part; else `""`.

### 11.6 `extract_user_prompt(payload) -> String`

Text of the **last** user message, with a leading `[...]` bracket prefix stripped
(hand-rolled scan equivalent to `^\[[^\]]+\]\s*` — no regex crate needed).

### 11.7 Reasoning caps auto-think

After model resolution: if catalog `capabilities.reasoning.supported == true` and
`can_disable == false`, set `payload.thinking = {"type": "adaptive"}`.

---

## 12. Conversation Tracking

Purpose: key affinity (same conversation → same key) and session numbering for logs.

### 12.1 Structure

```rust
struct Session { token_index: usize, request_count: u64, sess_num: u64 }

// True LRU (the Go spec noted Go maps can't do this correctly; Rust can):
struct ConvMap {
    inner: Mutex<lru::LruCache<String, Session>>,  // capacity 10_000
    counter: AtomicU64,                            // global session counter
}
```

### 12.2 Semantics

- `touch(fp)`: `get_mut` (promotes to MRU), return copy if present.
- `track(fp, session)`: `put` (promotes/creates). When at capacity and inserting a
  new fingerprint, evict down to **80% of max (8,000)** before insert — pop LRU in
  a loop. (Matches the source's batch-evict behavior rather than one-at-a-time.)
- New fingerprint → `sess_num = counter.fetch_add(1) + 1`, `request_count = 1`.
- Existing → `request_count += 1`, update `token_index` to the acquired key.
- On `request_count == 1`, log the first prompt (80-char truncated) to stderr:
  `[session {n}] first prompt: {text}` — both pipelines.

### 12.3 Flow per request

1. fingerprint → `touch`
2. acquire key: preferred = cached `token_index` if session exists, else round-robin
3. create/update session; log first prompt if new.

---

## 13. Retry Logic (OpenAI path only)

### 13.1 Loop

Up to `MAX_RETRIES = 10` attempts. Delay before attempt *n+1*:
`3000ms + 3000ms * (n - 1)` → 3 s, 6 s, 9 s, … Use `tokio::time::sleep`.

### 13.2 Retryable

- HTTP **500** and **503** (any body)
- Network/connect/timeout errors before a response (treated as 502)

### 13.3 Key rotation

On each retryable failure mark the current key unhealthy (with the status). On
attempts > 1, if `pool.total() > 1`, acquire a fresh slot before retrying.

### 13.4 Non-retryable

Any other HTTP status (400/401/404/429/…) is passed through immediately.

### 13.5 Throttle bump

On upstream **429 or 503** (final or non-retryable), increment the throttled
counter. Applies to both pipelines.

### 13.6 Body reuse

The serialized payload `Bytes` is cloned per attempt (refcount bump) — the payload
is **not** re-serialized on retry.

---

## 14. Error Logging

- Directory `.logs/` auto-created; file `errors-{timestamp}.log` with `:` and `.`
  replaced by `-`; opened once per process, appended via a `Mutex<BufWriter<File>>`
  (or a dedicated logging task fed by an `mpsc` channel to keep the hot path clean).
- Record format: `--- HTTP ERROR ---\n{json}\n\n` containing timestamp, error type
  (`upstream_http_error`), stage (`retryable_attempt` | `final_attempt`), attempt,
  session (`sessNum`, `slotName`), request (method, url, redacted headers, redacted
  body), upstream (url, method, redacted headers, status, status text, redacted body).

### 14.1 `redact_headers`

Replace with `[REDACTED]` when the lowercase name matches exactly
`authorization|x-api-key|cookie|set-cookie|api-key` or contains
`auth|token|key|password|secret`.

### 14.2 `redact_body_json`

Parse as JSON; walk: redact values of keys (case-insensitive)
`api_key|apikey|token|password|secret|authorization`; recurse into `messages`;
truncate `content` strings > 2000 chars to 2000 + `...[truncated]`. Non-JSON →
as-is; serialization failure → `[unserializable]`.

---

## 15. HTTP Routes

| Route | Method | Auth | Description |
|---|---|---|---|
| `/`, `/dashboard` | GET | No | Serve `dashboard.html` (§25) |
| `/healthz` | GET | No | Health check |
| `/api/config` | GET | Yes | Read config (api key masked) |
| `/api/config` | POST | Yes | Update config fields (§27) |
| `/api/validate` | GET | Yes | Validate active api key |
| `/api/models` | GET | Yes | Dashboard model list + disabled + display names |
| `/api/keys` | GET | Yes | List keys (masked) |
| `/api/keys` | POST | Yes | Add / update / delete keys (§28) |
| `/api/umans/usage` | GET | Yes | Usage data (`?fresh=1` bypasses cache) |
| `/api/umans/concurrency` | GET | Yes | Concurrency data (`?fresh=1`) |
| `/api/umans/usage-history` | GET | Yes | Proxied `/usage/history` (§29) |
| `/api/umans/user` | GET | Yes | `{loggedIn, email:"", user_id}` |
| `/api/restart` | POST | Yes | Graceful exit code 42 (§31) |
| `/api/bg` | GET | Yes | Bing wallpaper (§30) |
| `/api/bg-wallhaven` | GET | Yes | Wallhaven wallpaper (§30) |
| `/v1/models` | GET | Yes | OpenAI-format model list + pricing |
| `/v1/models/info` | GET | Yes | Raw catalog snapshot (`info` map) |
| `/v1/chat/completions` | POST | Yes | OpenAI pipeline (gated) |
| `/v1/messages`, `/messages` | POST | Yes | Anthropic pipeline (gated) |

Everything else → 404. `/api/bg-freegen` returns 404 explicitly (FreeGen excluded).

### 15.1 Authentication

- `config.api_keys` empty → open access.
- Accept `X-Api-Key: <k>` or `Authorization: Bearer <k>` where `k ∈ api_keys`.
- Else 401 in the format matching the route (OpenAI vs Anthropic error shape).
- Compare with constant-time equality (`subtle` crate or length-then-ct compare)
  — cheap hardening, localhost or not.

### 15.2 Body reading

`http_body_util::Limited` with `MAX_BODY_SIZE = 5 MiB`; overflow → 400. Collect to
`Bytes` once.

### 15.3 Response writers

A small `RespWriter` abstraction owning the streaming body sender and a
`headers_sent: bool`:

- `write_json(status, value)` — if headers already sent (SSE keepalive case),
  emit the value as an SSE `data:` frame and close; else normal JSON response.
- `write_openai_error(status, msg, type, code?)` → `{"error":{"message","type","code"?}}`
- `write_anthropic_error(status, msg, type?)` → `{"type":"error","error":{"type","message"}}`
  with type defaulted from status: 400 `invalid_request_error`, 401
  `authentication_error`, 403 `permission_error`, 404 `not_found_error`, 429
  `rate_limit_error`, 500 `api_error`, 503 `overloaded_error`.
- Passthrough variants parse the upstream error body and re-shape:
  OpenAI: message = `error.message | message`, type = `error.type | "upstream_error"`,
  code = `error.code`. Anthropic: type default `api_error`.

---

## 16. OpenAI Pipeline (`/v1/chat/completions`)

1. Auth → read body (≤ 5 MiB) → parse to `Value` (single parse).
2. Gate admission (§5.2). All following steps run with the permit held.
3. Fingerprint → session touch → key acquire (preferred index; 503 OpenAI-error if
   no healthy keys) → session create/update; log first prompt.
4. `strip_reasoning_content`.
5. `resolve_model_id`; write back to `payload.model`.
6. Tool normalization (guarded fast path, §10.1).
7. If **not** a handoff model: `limit_images_in_messages(max_images)`.
8. Reasoning caps auto-think (§11.7).
9. `normalize_thinking_payload`.
10. If handoff needed **and** client requested streaming: flush SSE headers +
    keepalive comment (§9.6).
11. `perform_vision_handoff`.
12. Serialize payload once → `Bytes`.
13. **Retry loop** (§13):
    - attempt > 1 and pool > 1 → fresh key.
    - network error → mark unhealthy(502); retry, or 502 passthrough on last.
    - 2xx:
      - mark key healthy.
      - upstream `Content-Type` is SSE → stream frames to the client
        (§18 piping). Done.
      - JSON → read body; if the client asked for streaming but got JSON, wrap the
        parsed response as a single SSE chunk + `data: [DONE]`; else write JSON
        through.
    - 500/503 → mark unhealthy, log (§14), retry; on last attempt pass the error
      through (and bump throttled if 503).
    - other status → bump throttled if 429; pass through immediately.

---

## 17. Anthropic Pipeline (`/v1/messages`, `/messages`)

Pass-through: **no retry loop, no response cache**.

1. Auth → body → parse. Nil-guards: if pool/upstream unavailable → 503/500
   Anthropic error (never panic; in Rust this falls out of `Option` handling).
2. Gate admission; fingerprint/session/key acquire as in §16 (Anthropic error
   shapes); log first prompt for new sessions.
3. `normalize_thinking_payload`; `resolve_model_id` → `payload.model`;
   `limit_images_in_messages`; `perform_vision_handoff` (with SSE keepalive if
   streaming).
4. Serialize once → `upstream.messages(...)`.
5. Status ≥ 400: log; `write_anthropic_passthrough_error`; mark unhealthy **only
   on 503** (500 is usually a payload problem); bump throttled on 429/503.
6. Success: mark healthy; propagate upstream status + `Content-Type`, add
   `Cache-Control: no-cache`; pipe body (§18).
7. Network error: passthrough 502 Anthropic error; mark unhealthy(502).
8. Response body lifetime is scope-managed (RAII) — the Go spec's manual
   double-close concern does not exist in Rust.

---

## 18. Streaming & Piping

`pipe(upstream_body, client_sender)`:

- Forward `Bytes` frames as they arrive; **no buffering, no copying** (frames are
  refcounted).
- **Backpressure** is inherent: `send().await` on the body channel suspends until
  the client consumes.
- **Client disconnect**: hyper drops the response body/sender; the send fails →
  drop the upstream body (aborts the upstream connection). Additionally run the
  copy inside the connection's task scope so cancellation propagates.
- Clean EOF = stream end (`None` frame); map upstream mid-stream errors to
  terminating the client stream (cannot change status after headers).

---

## 19. `/healthz`

No auth. Returns:

```json
{
  "ok": true,
  "started_at": "ISO-8601",
  "uptime_sec": 12345,
  "api_key_valid": true,
  "provider": "umans",
  "token_state": [ /* §3.7 */ ],
  "valid_tokens": 2,
  "total_tokens": 3,
  "models_count": 15,
  "runtime": "rust",
  "runtime_version": "rustc 1.xx",
  "port": 8084,
  "visionHandoff": {
    "enabled": false,
    "cacheEnabled": false,
    "cache": { "size": 0, "maxSize": 50, "ttlMs": 86400000, "hits": 0, "misses": 0, "evictions": 0 }
  }
}
```

- Uses cached user info; refresh if older than 5 min.
- `runtime_version` embedded at build time (`env!` via build script or
  `rustc --version` baked in).

---

## 20. `/v1/models`

OpenAI-format list over `effective_models()`:

- Base: `id`, `object: "model"`, `created` = server start unix ts,
  `owned_by: "umans"`, `root` = id, `permission: []`.
- `display_name` from the catalog display map (fallback: strip `umans-` prefix).
- If `capabilities.context_window` is a positive number:
  - `context_length = context_window`
  - `max_output_tokens` = first of `recommended_max_tokens`,
    `max_completion_tokens`, `context_window` — **top-level field**, not nested
    (avoids key collisions in downstream pricing extractors).
- If upstream pricing (§8.5) has the model:
  - `pricing.prompt = input / 1_000_000`, `pricing.completion = output / 1_000_000`
    (only when the source values are numbers; omit missing keys).

`/v1/models/info` returns the raw catalog `info` map as JSON.

### 20.1 Dashboard endpoints

- `GET /api/validate` (auth): runs `validate_api_key()` (§8.6); returns
  `{ "valid": bool }` (and refreshes the catalog snapshot on success).
- `GET /api/models` (auth): the dashboard's model view. For each id from
  `all_catalog_models()` (§8.4 — includes disabled so the dashboard can render a
  toggle), attach the fields it needs, all derived from the umans catalog (§8.7):
  ```json
  {
    "models": [
      {
        "id": "umans-coder",
        "displayName": "Coder",
        "reasoning": true,
        "variants": { "low": {"thinking": {"type":"enabled","budget_tokens":8000}}, "...": {} },
        "supportsTools": true,
        "supportsVision": "via-handoff",
        "contextWindow": 200000
      }
    ],
    "disabled": ["...config.disabled_models..."],
    "displayNames": { "umans-coder": "Coder" }
  }
  ```
  `reasoning` = `reasoning_mode(caps)`, `variants` = `reasoning_variants(caps)`
  (omitted when `None`). No models.dev lookup anywhere in this path.

---

## 21. Startup Sequence

1. Load config + env overrides; validate.
2. Init `HandoffCache` (50 entries; configured TTL).
3. Init `KeyPool` from `config.keys` (or single default from `API_KEY`).
4. Init upstream client.
5. `validate_api_key()` → seeds catalog snapshot (non-fatal on failure; log).
6. `fetch_concurrency()` → size the Gate.
7. Bind `TcpListener` on `LISTEN_ADDR` with **port retry**: on `EADDRINUSE`, retry
   the same port 3× (2 s apart), then increment the port and reset the retry count.
8. Preload `dashboard.html` into the mtime cache (§24.1); warm the wallpaper cache
   in the background if `wallpaper_source != none`.
9. Serve. Spawn: the debounced config-save task (§2.8) and a background task
   refreshing usage/concurrency on the 5-min cadence (and reconciling the Gate) so
   gating tracks upstream limits without request traffic.
10. Optional first-run browser open (new config file): `xdg-open` / `open` / `start`
    on the dashboard URL. Off by default; enable with `--open` or `OPEN_DASHBOARD=1`.
11. Install SIGINT/SIGTERM handler.

---

## 22. Shutdown

On SIGINT/SIGTERM:

1. Log signal; stop accepting (drop the listener / use hyper-util graceful shutdown).
2. **Drain**: wait until `active == 0`, polling every 100 ms, max 5 s
   (or `tokio_util::sync::CancellationToken` + a `TaskTracker` for exactness).
3. Force-shutdown remaining connections after the 5 s budget.
4. Flush + close the error log.
5. Exit 0.

Handler panics must not kill the process: wrap handler futures with
`catch_unwind` (via `tower_http::catch_panic` or manual `AssertUnwindSafe`) →
500 in the route-appropriate error shape.

---

## 23. Constants

```rust
pub const UMANS_API_BASE: &str        = "https://api.code.umans.ai/v1";
pub const API_KEY_ENV_VAR: &str       = "UMANS_API_KEY";
pub const MAX_RETRIES: u32            = 10;
pub const RETRY_DELAY: Duration       = Duration::from_secs(3);
pub const MAX_QUEUE_SIZE: usize       = 256;
pub const MAX_BODY_SIZE: usize        = 5 * 1024 * 1024;
pub const CONVERSATION_MAP_MAX: usize = 10_000;
pub const CATALOG_CACHE_TTL: Duration = Duration::from_secs(300);
pub const USAGE_CACHE_TTL: Duration   = Duration::from_secs(300);
pub const DEFAULT_KEY_COOLDOWN: Duration = Duration::from_secs(30);
pub const HANDOFF_CACHE_SIZE: usize   = 50;
pub const HANDOFF_CACHE_TTL: Duration = Duration::from_secs(86_400);
```

Reasoning-level budgets (§8.7), preserved verbatim from the Go build (note
`medium == high == 16000` is intentional, matching the source):

```rust
// &[(&str, u32)] or a phf/once_cell map
pub const REASONING_LEVEL_BUDGETS: &[(&str, u32)] = &[
    ("low",    8_000),
    ("medium", 16_000),
    ("high",   16_000),
    ("max",    32_000),
];
```

---

## 24. Dashboard Serving

### 24.1 `get_dashboard_html() -> Option<Bytes>`

Read `dashboard.html` from the binary's directory. Cache by mtime: keep
`ArcSwap<Option<(SystemTime, Bytes)>>`; if the file's mtime is unchanged, return the
cached `Bytes` (refcount clone, no re-read). On read error → `None`.

### 24.2 Route handler (`/`, `/dashboard`)

1. Get HTML (cached or fresh); `None` → 404.
2. Inject a wallpaper `<style>` before `</head>` based on `config.wallpaper_source`:
   - `none` → `<style>body{background:#0d1117}</style>`.
   - `bing` / `wallhaven` → if the cached wallpaper file exists
     (`.cache/wallpaper.jpg` or `.cache/wallpaper-haven.jpg`), embed it base64 as a
     `background-image` (prevents white flash); otherwise fall back to the dark bg.
3. Serve `text/html`. No auth (matches the Go route table).

Injection is a single `.replace("</head>", &format!("{style}</head>"))` on a
`Cow<str>` view of the cached bytes; the base64 embed is computed once per wallpaper
refresh and cached alongside the `.cache` file so each dashboard hit is allocation-light.

---

## 25. dashboard.html (ported feature set)

A standalone `dashboard.html` served at `/` and `/dashboard`, unchanged in spirit
from the Go build (Bootstrap 5 + Icons via CDN, procedural glassmorphism). Ported
cards/controls:

- **5-hour Window** card: requests, throttled, cached %, error %, start time,
  tokens in/out, plan badge.
- **Current Concurrency** card: active, queued, limit, burst, dual-fill progress bar
  (soft-cap / burst zones), upstream overlay, percentage, detail grid.
- **Usage History** card: bar chart (Y labels, dashed grid, X labels), click-to-filter
  table, expandable per-model breakdown, sortable headers, Tokens/Requests toggle,
  status legend.
- **User ID** in header (click-to-reveal masking).
- **API Key** section: key-pool display with status badges, collapsible.
- **Models** section: per-model enable/disable toggle (drives `DISABLED_MODELS`),
  collapsible.
- **Quick Settings**: auto-refresh interval, wallpaper selector (None/Bing/Wallhaven),
  vision-handoff toggle (+ info tooltip), handoff-cache toggle (shown only when
  handoff on) with cache-stats line.
- **Quick Actions**: check health, test connection, manual refresh, restart proxy.
- **Environment**: runtime (`rust`), port, started-at.
- **Key Management modal**: add/edit/delete keys inline, account info with user id.
- Auto-refresh: status 15 s, usage on the configured interval, concurrency 15 s,
  usage history 5 min. Dashboard always fetches usage/concurrency with `?fresh=1`.
- Toast notifications.

Excluded (as in the Go build): FreeGen prompt/generate, Sleev context-compression
toggle, FreeGen wallpaper option.

The HTML itself is a static asset shipped next to the binary — not generated by the
Rust code. This spec covers only the endpoints it calls.

---

## 26. Config API — `GET /api/config`

Auth required. Returns config with the api key masked:

```json
{
  "listenAddr": "127.0.0.1:8084",
  "upstreamBaseURL": "https://api.code.umans.ai/v1",
  "apiKey": "sk-1234567...wxyz",
  "enabledModels": [],
  "modelDisplayNames": {},
  "wallpaperSource": "bing",
  "overrideConcurrency": 0,
  "maxImages": 9,
  "disabledModels": [],
  "visionHandoffEnabled": false,
  "visionHandoffModel": "umans-coder",
  "visionHandoffPrompt": "",
  "visionHandoffCacheEnabled": false
}
```

Serialize from the current `config.load()` snapshot; mask via `mask_token` (§2.6).

---

## 27. Config API — `POST /api/config`

Auth required. Accepts a **partial** update (all fields optional); apply each present
field over a clone of the current snapshot, then `store()` the new `Arc<Config>` and
ping `debounced_save` (§2.8). Updatable: `apiKey`, `apiKeys`, `listenAddr`,
`enabledModels`, `modelDisplayNames`, `wallpaperSource` (`none`|`bing`|`wallhaven`),
`overrideConcurrency`, `maxImages`, `disabledModels`, `visionHandoffEnabled`,
`visionHandoffModel`, `visionHandoffPrompt`, `visionHandoffCacheEnabled`, `keys`
(rebuilds the key pool via §3.8).

Side effects on specific fields:
- `overrideConcurrency` → recompute effective concurrency + reconcile the Gate (§6.3, §5.2).
- `visionHandoffCacheEnabled` / cache TTL → `HandoffCache::resize` (§4.2).
- `keys` → `KeyPool::rebuild` (§3.8).

**Response**: `{ ..updated fields.., "restartRequired": bool }`. `restartRequired`
is `true` **iff** `listenAddr` changed (the listener can't rebind live; the dashboard
surfaces a restart prompt, actioned via §31).

Deserialize the partial with a struct of `Option<T>` fields
(`#[serde(default)]`); `None` means "leave as-is". This avoids the JS/Go ambiguity
between "field absent" and "field set to zero value".

---

## 28. Key Management API

### 28.1 `GET /api/keys`

```json
{
  "keys": [ /* raw entries */ ],
  "safe": [
    { "name": "Default", "token_masked": "sk-1234...wxyz", "has_token": true, "has_session": false }
  ]
}
```

### 28.2 `POST /api/keys`

Body carries an `action` and payload:
- `add`: push `{name, key, session:""}`; if no `config.apiKey` is set, set it to this
  key. Rebuild pool.
- `update`: update entry at `index`; if `index == 0` and a key is present, set it as
  `config.apiKey`. Rebuild pool.
- `delete`: remove entry at `index`; if the list empties, push placeholder
  `{name:"Key 1", key:"", session:""}`; if `index == 0`, refresh `config.apiKey`.
  Rebuild pool.

All actions `store()` the new config and ping `debounced_save`. Pool rebuild is the
live swap from §3.8 (in-flight requests unaffected).

---

## 29. Usage API Endpoints

All auth-required; all read from the §6 usage/concurrency layer.

### 29.1 `GET /api/umans/usage`

```json
{
  "usage": { "requests_in_window": 246, "tokens_in": 24000000, "tokens_out": 11732073, "tokens_cached": 9360000 },
  "window": { "started_at": "..." },
  "plan": { "display_name": "..." },
  "throttled": 0
}
```
`?fresh=1` bypasses the 5-min cache. `throttled` is the proxy-side queue-full 503 count.

### 29.2 `GET /api/umans/concurrency`

```json
{ "concurrent": 3, "limit": 8, "hard_cap": 16, "user_id": "...", "overridden": false, "active": 2, "queued": 1 }
```
`?fresh=1` bypasses. `active` / `queued` come straight from the Gate atomics (§5.1).

### 29.3 `GET /api/umans/usage-history`

Proxy of upstream `GET {base}/usage/history` with a 5-min cache (`?fresh=1`
bypasses). Forward query params `from`, `to`, `granularity`, `scope`. Cache key =
the normalized query string. Return the upstream JSON verbatim.

### 29.4 `GET /api/umans/user`

`{ "loggedIn": true, "email": "", "user_id": "..." }` — `user_id` from the stored
`LastConcurrency.user_id` (§6.2).

---

## 30. Wallpaper Proxies

Both cache to `.cache/` and reuse a shared image-download helper (browser
User-Agent, streaming download to a temp file + atomic rename).

### 30.1 `GET /api/bg` — Bing (daily)

1. If `.cache/wallpaper.jpg` exists and was modified **today** (UTC) → serve it
   (`image/jpeg`).
2. Else: `GET https://peapix.com/bing/feed` (browser UA, 15 s); parse JSON; take the
   first item's `fullUrl` (or `imageUrl`/`url`); download (30 s); save; serve.
3. On fetch error: serve the stale cached file if present, else 500.
4. Set `Expires` to end of today (UTC). TTL 24 h.

### 30.2 `GET /api/bg-wallhaven` — Wallhaven (hourly)

1. If `.cache/wallpaper-haven.jpg` exists and is **< 1 h old** → serve it.
2. Else: `GET https://wallhaven.cc/api/v1/search?categories=100&purity=100&topRange=1M&sorting=toplist&order=desc&page=3`
   (`User-Agent: umans-proxy/1.0`, 15 s); parse JSON; pick a random entry from
   `data`; download its `path` (30 s); save; serve.
3. On fetch error: serve stale cache if present, else 500. TTL 1 h.

Concurrency: guard each wallpaper refresh with a per-source `tokio::sync::Mutex` so a
burst of dashboard loads triggers exactly one upstream fetch (single-flight); other
waiters serve the freshly cached file.

---

## 31. Restart API — `POST /api/restart`

1. Respond `{ "success": true, "message": "Restarting..." }`.
2. After 500 ms (`tokio::spawn` + `sleep`): begin graceful shutdown (§22) and exit
   with **code 42**. An external supervisor (systemd `RestartForceExitStatus=42`,
   or a wrapper script) restarts the process — this is how a `listenAddr` change
   (§27) actually takes effect.

---

## 32. Removed vs. the Go spec

| Removed | Why |
|---|---|
| Opencode config discovery/setup, debounced opencode writers, first-run browser open | Client-harness integration; not wanted |
| models.dev catalog, provider mapping, model-id candidates | Enrichment/mapping layer; only needed to generate opencode configs |

Reasoning metadata is **not** lost with models.dev: reasoning mode and per-level
variants derive directly from the umans catalog's `capabilities.reasoning` plus the
`REASONING_LEVEL_BUDGETS` table (§8.7, §23). models.dev only ever supplied an
optional `reasoning_options` preference and the opencode provider mapping — both
gone with opencode.

Already absent in the Go source and still absent here: Sleev context-compression
gateway, FreeGen wallpaper generation, the general response cache, and the
`/api/sleev` · `/api/bg-freegen` · `/api/cache` · `/api/i18n` endpoints. Everything
else from the Go spec — dashboard, `/api/*`, wallpaper proxies, restart — is present
with behavior-compatible semantics.

> Note: because `POST /api/config` and `POST /api/keys` mutate config at runtime,
> §2.7 makes config a runtime-swappable `ArcSwap` snapshot with debounced
> persistence (§2.8) — the one place the "kept" dashboard changes an otherwise
> read-only core.

---

## 33. Suggested module layout

```
src/
  main.rs            // startup, shutdown, signal handling
  config.rs          // §2
  keypool.rs         // §3
  handoff_cache.rs   // §4
  gate.rs            // §5
  usage.rs           // §6
  upstream.rs        // §7
  catalog.rs         // §8
  vision.rs          // §9
  schema_norm.rs     // §10
  payload.rs         // §11
  sessions.rs        // §12
  retry.rs           // §13
  errlog.rs          // §14
  persist.rs         // §2.8 debounced config save
  routes/
    mod.rs           // router + auth (§15)
    chat.rs          // §16
    messages.rs      // §17
    stream.rs        // §18 (piping)
    healthz.rs       // §19
    models.rs        // §20  (/v1/models, /v1/models/info)
    dashboard.rs     // §24  (/, /dashboard + html mtime cache)
    config_api.rs    // §26–27  (/api/config)
    keys_api.rs      // §28  (/api/keys)
    usage_api.rs     // §29  (/api/umans/*)
    wallpaper.rs     // §30  (/api/bg, /api/bg-wallhaven)
    restart.rs       // §31  (/api/restart)
```

`dashboard.html` ships as a sibling asset next to the binary (not embedded), so it
can be edited without a recompile — matching the Go build's mtime-cached serving.

### Testing notes

- `schema_norm`, `payload`, and `vision::collect_image_parts` are pure
  `Value → Value` transforms: golden-file unit tests against fixtures captured
  from real opencode/Claude traffic.
- Gate: loom-style or plain tokio tests for admission, queue-full 503, and
  resize reconciliation.
- Retry: mock upstream (`wiremock`) exercising 500→success, 503×10→passthrough,
  network-error→502, key rotation order.
