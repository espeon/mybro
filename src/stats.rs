// ── In-memory time-series request stats ──────────────────────────────────────
//
// Records per-request data into a ring buffer, then aggregates into time
// buckets for the dashboard. No external storage — this is for the live
// "last N minutes" view, like a mini-Grafana.

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use tokio::sync::mpsc;

// ── Per-request record ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct RequestRecord {
    pub ts_ms: u64,
    pub duration_ms: u64,
    /// Time to first byte (or chunk, for streaming). None if not yet observed.
    pub ttft_ms: Option<u64>,
    pub status: u16,
    pub model: String,
    pub pipeline: &'static str, // "openai" | "anthropic"
    pub key_name: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    /// Number of input tokens that were served from upstream cache (cache hits)
    pub cached_tokens: u64,
    /// Number of input tokens that were written to cache this request (cache warming)
    pub cache_creation_tokens: u64,
    pub cached: bool,
    pub error: Option<String>,
}

// ── Ring buffer ──────────────────────────────────────────────────────────────

const DEFAULT_MAX_RECORDS: usize = 10_000;

pub struct StatsCollector {
    records: Mutex<VecDeque<RequestRecord>>,
    max_records: usize,
    db_sender: Option<mpsc::Sender<RequestRecord>>,
    db: Option<Arc<crate::db::StatsDB>>,
}

/// Windows shorter than this use the in-memory buffer; longer windows query SQLite.
const MEMORY_WINDOW_MS: u64 = 5 * 60 * 1000; // 5 minutes

impl StatsCollector {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_MAX_RECORDS)
    }

    pub fn with_capacity(max: usize) -> Self {
        Self {
            records: Mutex::new(VecDeque::with_capacity(max)),
            max_records: max,
            db_sender: None,
            db: None,
        }
    }

    pub fn with_db(db: Arc<crate::db::StatsDB>) -> Self {
        Self {
            records: Mutex::new(VecDeque::with_capacity(DEFAULT_MAX_RECORDS)),
            max_records: DEFAULT_MAX_RECORDS,
            db_sender: Some(db.sender()),
            db: Some(db),
        }
    }

    pub fn record(&self, rec: RequestRecord) {
        // Persist to SQLite asynchronously
        if let Some(sender) = &self.db_sender {
            let _ = sender.try_send(rec.clone());
        }
        let mut records = self.records.lock();
        if records.len() >= self.max_records {
            records.pop_front();
        }
        records.push_back(rec);
    }

    /// Aggregate into time buckets of `bucket_ms` duration, covering the last
    /// `window_ms` of data. Returns buckets oldest→newest. Two-pass: first
    /// counts/sums, then collects latencies per bucket for percentiles.
    pub fn buckets(&self, window_ms: u64, bucket_ms: u64, model: Option<&str>) -> Vec<StatsBucket> {
        // Use SQLite for windows longer than memory coverage, or if no DB just use memory
        if let Some(db) = &self.db {
            if window_ms > MEMORY_WINDOW_MS {
                match db.query_buckets(window_ms, bucket_ms, model) {
                    Ok(buckets) => return buckets,
                    Err(e) => tracing::warn!("stats db query failed: {}", e),
                }
            }
        }
        let records = self.records.lock();
        let now_ms = now_millis();
        let start_ms = now_ms.saturating_sub(window_ms);
        let num_buckets = ((window_ms / bucket_ms).max(1)) as usize;

        // Initialize empty buckets
        let mut buckets: Vec<StatsBucket> = (0..num_buckets)
            .map(|i| StatsBucket {
                ts_ms: start_ms + (i as u64) * bucket_ms,
                count: 0,
                errors: 0,
                throttled: 0,
                avg_latency_ms: 0.0,
                p50_latency_ms: 0.0,
                p95_latency_ms: 0.0,
                max_latency_ms: 0,
                avg_ttft_ms: 0.0,
                p50_ttft_ms: 0.0,
                p95_ttft_ms: 0.0,
                tokens_in: 0,
                tokens_out: 0,
                cached: 0,
                cached_tokens: 0,
                cache_creation_tokens: 0,
                cache_hit_rate: 0.0,
                by_model: BTreeMap::new(),
            })
            .collect();

        // Latencies and TTFTs collected per bucket for percentile computation
        let mut latencies_per_bucket: Vec<Vec<u64>> = vec![Vec::new(); num_buckets];
        let mut ttfts_per_bucket: Vec<Vec<u64>> = vec![Vec::new(); num_buckets];

        // Single pass over records
        for rec in records.iter() {
            if rec.ts_ms < start_ms || rec.ts_ms > now_ms {
                continue;
            }
            if let Some(m) = model {
                if rec.model != m {
                    continue;
                }
            }
            let bi = ((rec.ts_ms - start_ms) / bucket_ms) as usize;
            if bi >= num_buckets {
                continue;
            }
            let bucket = &mut buckets[bi];
            bucket.count += 1;
            if rec.status >= 400 {
                bucket.errors += 1;
            }
            if rec.status == 429 || rec.status == 503 {
                bucket.throttled += 1;
            }
            bucket.max_latency_ms = bucket.max_latency_ms.max(rec.duration_ms);
            bucket.tokens_in += rec.tokens_in;
            bucket.tokens_out += rec.tokens_out;
            bucket.cached_tokens += rec.cached_tokens;
            bucket.cache_creation_tokens += rec.cache_creation_tokens;
            if rec.cached || rec.cached_tokens > 0 || rec.cache_creation_tokens > 0 {
                bucket.cached += 1;
            }

            let model_entry = bucket
                .by_model
                .entry(rec.model.clone())
                .or_default();
            model_entry.count += 1;
            model_entry.latency_sum_ms += rec.duration_ms;
            model_entry.tokens_in += rec.tokens_in;
            model_entry.tokens_out += rec.tokens_out;

            latencies_per_bucket[bi].push(rec.duration_ms);
            if let Some(ttft) = rec.ttft_ms {
                ttfts_per_bucket[bi].push(ttft);
            }
        }

        // Compute averages and percentiles
        for (i, bucket) in buckets.iter_mut().enumerate() {
            if bucket.count == 0 {
                continue;
            }
            let lats = &latencies_per_bucket[i];
            let sum: u64 = lats.iter().sum();
            bucket.avg_latency_ms = sum as f64 / lats.len() as f64;

            let mut sorted = lats.clone();
            sorted.sort_unstable();
            bucket.p50_latency_ms = percentile(&sorted, 50.0);
            bucket.p95_latency_ms = percentile(&sorted, 95.0);

            let ttfts = &ttfts_per_bucket[i];
            if !ttfts.is_empty() {
                let t_sum: u64 = ttfts.iter().sum();
                bucket.avg_ttft_ms = t_sum as f64 / ttfts.len() as f64;
                let mut t_sorted = ttfts.clone();
                t_sorted.sort_unstable();
                bucket.p50_ttft_ms = percentile(&t_sorted, 50.0);
                bucket.p95_ttft_ms = percentile(&t_sorted, 95.0);
            }

            // Cache hit rate: fraction of input tokens that were cache hits
            if bucket.tokens_in > 0 {
                bucket.cache_hit_rate = bucket.cached_tokens as f64 / bucket.tokens_in as f64;
            }
        }

        buckets
    }

    /// Get recent raw records (for a table view), newest first.
    pub fn recent(&self, limit: usize, model: Option<&str>) -> Vec<RequestRecord> {
        let records = self.records.lock();
        records
            .iter()
            .rev()
            .filter(|r| model.map_or(true, |m| r.model == m))
            .take(limit)
            .cloned()
            .collect()
    }

    /// Summary stats for the last `window_ms`.
    pub fn token_stats(&self, window_ms: u64, model: Option<&str>) -> Vec<crate::db::TokenSummary> {
        if let Some(db) = &self.db {
            match db.query_token_stats(window_ms, model) {
                Ok(tokens) => return tokens,
                Err(e) => tracing::warn!("token stats query failed: {}", e),
            }
        }
        // Fallback: in-memory aggregation
        let records = self.records.lock();
        let now_ms = now_millis();
        let start_ms = now_ms.saturating_sub(window_ms);
        let mut by_key: std::collections::BTreeMap<String, crate::db::TokenSummary> =
            std::collections::BTreeMap::new();
        for r in records
            .iter()
            .filter(|r| r.ts_ms >= start_ms)
            .filter(|r| model.map_or(true, |m| r.model == m))
        {
            let e = by_key.entry(r.key_name.clone()).or_insert_with(|| {
                crate::db::TokenSummary {
                    key_name: r.key_name.clone(),
                    count: 0,
                    errors: 0,
                    tokens_in: 0,
                    tokens_out: 0,
                    avg_latency_ms: 0.0,
                }
            });
            e.count += 1;
            if r.status >= 400 {
                e.errors += 1;
            }
            e.tokens_in += r.tokens_in;
            e.tokens_out += r.tokens_out;
        }
        for e in by_key.values_mut() {
            e.avg_latency_ms = 0.0;
        }
        by_key.into_values().collect()
    }

    /// Distinct model names seen in the last `window_ms`, for populating
    /// the dashboard filter dropdown.
    pub fn distinct_models(&self, window_ms: u64) -> Vec<String> {
        if let Some(db) = &self.db {
            if window_ms > MEMORY_WINDOW_MS {
                match db.query_distinct_models(window_ms) {
                    Ok(models) => return models,
                    Err(e) => tracing::warn!("distinct models query failed: {}", e),
                }
            }
        }
        let records = self.records.lock();
        let now_ms = now_millis();
        let start_ms = now_ms.saturating_sub(window_ms);
        let mut models: Vec<String> = records
            .iter()
            .filter(|r| r.ts_ms >= start_ms)
            .map(|r| r.model.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        models.sort();
        models
    }

    pub fn summary(&self, window_ms: u64, model: Option<&str>) -> StatsSummary {
        if let Some(db) = &self.db {
            if window_ms > MEMORY_WINDOW_MS {
                match db.query_summary(window_ms, model) {
                    Ok(summary) => return summary,
                    Err(e) => tracing::warn!("stats db summary failed: {}", e),
                }
            }
        }
        let records = self.records.lock();
        let now_ms = now_millis();
        let start_ms = now_ms.saturating_sub(window_ms);

        let relevant: Vec<&RequestRecord> = records
            .iter()
            .filter(|r| r.ts_ms >= start_ms)
            .filter(|r| model.map_or(true, |m| r.model == m))
            .collect();

        let count = relevant.len() as u64;
        let errors = relevant.iter().filter(|r| r.status >= 400).count() as u64;
        let throttled = relevant
            .iter()
            .filter(|r| r.status == 429 || r.status == 503)
            .count() as u64;
        let cached = relevant.iter().filter(|r| r.cached).count() as u64;
        let cached_tokens: u64 = relevant.iter().map(|r| r.cached_tokens).sum();
        let cache_creation_tokens: u64 = relevant.iter().map(|r| r.cache_creation_tokens).sum();
        let tokens_in: u64 = relevant.iter().map(|r| r.tokens_in).sum();
        let tokens_out: u64 = relevant.iter().map(|r| r.tokens_out).sum();
        let max_context_tokens = relevant.iter().map(|r| r.tokens_in).max().unwrap_or(0);
        let cache_hit_rate = if tokens_in > 0 {
            cached_tokens as f64 / tokens_in as f64
        } else {
            0.0
        };
        let avg_latency = if count > 0 {
            relevant.iter().map(|r| r.duration_ms).sum::<u64>() as f64 / count as f64
        } else {
            0.0
        };

        let ttfts: Vec<u64> = relevant.iter().filter_map(|r| r.ttft_ms).collect();
        let avg_ttft = if !ttfts.is_empty() {
            ttfts.iter().sum::<u64>() as f64 / ttfts.len() as f64
        } else {
            0.0
        };

        // Generation throughput: output tokens divided by time actually spent
        // generating (duration after the first token). Only requests that
        // reported a TTFT and spent time past it can be measured; prompt input
        // (including cached tokens) is deliberately excluded.
        let (gen_tokens_out, gen_time_ms) =
            relevant.iter().fold((0u64, 0u64), |(tok, time), r| match r.ttft_ms {
                Some(ttft) if r.duration_ms > ttft => {
                    (tok + r.tokens_out, time + (r.duration_ms - ttft))
                }
                _ => (tok, time),
            });

        StatsSummary {
            count,
            errors,
            throttled,
            cached,
            cached_tokens,
            cache_creation_tokens,
            cache_hit_rate,
            tokens_in,
            tokens_out,
            avg_latency_ms: avg_latency,
            avg_ttft_ms: avg_ttft,
            max_context_tokens,
            gen_tokens_out,
            gen_time_ms,
        }
    }
}

impl Default for StatsCollector {
    fn default() -> Self {
        Self::new()
    }
}

fn percentile(sorted: &[u64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)] as f64
}

// ── Bucket types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct StatsBucket {
    pub ts_ms: u64,
    pub count: u64,
    pub errors: u64,
    pub throttled: u64,
    pub avg_latency_ms: f64,
    pub p50_latency_ms: f64,
    pub p95_latency_ms: f64,
    pub max_latency_ms: u64,
    pub avg_ttft_ms: f64,
    pub p50_ttft_ms: f64,
    pub p95_ttft_ms: f64,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cached: u64,
    pub cached_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_hit_rate: f64,
    pub by_model: BTreeMap<String, ModelBucket>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ModelBucket {
    pub count: u64,
    pub latency_sum_ms: u64,
    pub tokens_in: u64,
    pub tokens_out: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatsSummary {
    pub count: u64,
    pub errors: u64,
    pub throttled: u64,
    pub cached: u64,
    pub cached_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_hit_rate: f64,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub avg_latency_ms: f64,
    pub avg_ttft_ms: f64,
    /// Largest `tokens_in` value seen in the window. Useful as a cost watch —
    /// long contexts are expensive on some upstream models.
    pub max_context_tokens: u64,
    /// Output tokens across requests where generation time is measurable.
    /// Pairs with `gen_time_ms` to give generation throughput (tok/s).
    pub gen_tokens_out: u64,
    /// Summed generation time (`duration_ms - ttft_ms`) over those same requests.
    pub gen_time_ms: u64,
}

// ── Query params ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct StatsQuery {
    #[serde(default = "default_window")]
    pub window: u64,       // seconds
    #[serde(default = "default_bucket")]
    pub bucket: u64,       // seconds
    #[serde(default = "default_mode")]
    pub mode: String,      // "buckets" | "summary" | "recent"
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_window() -> u64 { 300 }
fn default_bucket() -> u64 { 10 }
fn default_mode() -> String { "buckets".to_string() }
fn default_limit() -> usize { 100 }

// ── Helpers ──────────────────────────────────────────────────────────────────

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record_at(ts_ms: u64, duration_ms: u64, ttft_ms: Option<u64>, tokens_out: u64) -> RequestRecord {
        RequestRecord {
            ts_ms,
            duration_ms,
            ttft_ms,
            status: 200,
            model: "big-model".to_string(),
            pipeline: "anthropic",
            key_name: "test".to_string(),
            tokens_in: 100_000, // large prompt: must NOT leak into throughput
            tokens_out,
            cached_tokens: 90_000,
            cache_creation_tokens: 0,
            cached: true,
            error: None,
        }
    }

    #[test]
    fn generation_throughput_uses_output_over_generation_time() {
        let collector = StatsCollector::new();
        let now = now_millis();
        // 80 tokens over 800ms of generation, 50 over 500ms → 130 tokens / 1.3s = 100 tok/s.
        collector.record(record_at(now - 1000, 1000, Some(200), 80));
        collector.record(record_at(now - 800, 600, Some(100), 50));
        // No TTFT reported → not measurable, excluded from both sums.
        collector.record(record_at(now - 500, 300, None, 30));

        let s = collector.summary(60_000, None);
        assert_eq!(s.gen_tokens_out, 130);
        assert_eq!(s.gen_time_ms, 1300);
        let throughput = s.gen_tokens_out as f64 / (s.gen_time_ms as f64 / 1000.0);
        assert!((throughput - 100.0).abs() < 1e-9, "got {throughput}");
    }

    #[test]
    fn generation_throughput_ignores_zero_length_generation() {
        let collector = StatsCollector::new();
        let now = now_millis();
        // duration == ttft: no measurable generation window.
        collector.record(record_at(now - 100, 200, Some(200), 40));

        let s = collector.summary(60_000, None);
        assert_eq!(s.gen_tokens_out, 0);
        assert_eq!(s.gen_time_ms, 0);
    }
}
