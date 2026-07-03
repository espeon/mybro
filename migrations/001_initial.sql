-- Initial schema for stats.db
--
-- Two tables:
--   - requests: raw per-request rows; rolled up on demand for short windows
--               and pre-aggregated into hourly_stats for long ranges.
--   - hourly_stats: pre-aggregated rows by (hour, model, key_name) for cheap
--                   long-window queries.

CREATE TABLE requests (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts_ms INTEGER NOT NULL,
    duration_ms INTEGER NOT NULL,
    ttft_ms INTEGER,
    status INTEGER NOT NULL,
    model TEXT NOT NULL,
    pipeline TEXT NOT NULL,
    key_name TEXT NOT NULL,
    tokens_in INTEGER NOT NULL,
    tokens_out INTEGER NOT NULL,
    cached_tokens INTEGER NOT NULL DEFAULT 0,
    cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
    cached INTEGER NOT NULL,
    error TEXT
);

CREATE INDEX idx_requests_ts    ON requests(ts_ms);
CREATE INDEX idx_requests_model ON requests(model);
CREATE INDEX idx_requests_key   ON requests(key_name);

CREATE TABLE hourly_stats (
    ts_hour INTEGER NOT NULL,
    model TEXT NOT NULL,
    key_name TEXT NOT NULL,
    count INTEGER NOT NULL,
    errors INTEGER NOT NULL,
    throttled INTEGER NOT NULL,
    latency_sum INTEGER NOT NULL,
    latency_max INTEGER NOT NULL,
    tokens_in INTEGER NOT NULL,
    tokens_out INTEGER NOT NULL,
    PRIMARY KEY (ts_hour, model, key_name)
);

CREATE INDEX idx_hourly_ts ON hourly_stats(ts_hour);