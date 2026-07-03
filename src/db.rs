// ── SQLite-backed long-term stats storage ──────────────────────────────────────
//
// Records are first buffered in memory (stats.rs) and drained here by a
// background task. This keeps the hot proxy path off disk.
//
// Schema:
//   requests(ts_ms, duration_ms, status, model, pipeline, key_name, tokens_in, tokens_out, cached, error)
//   hourly_stats(ts_hour, model, key_name, count, errors, throttled, latency_sum, latency_max, tokens_in, tokens_out)
//
// Retention: raw requests 24h; hourly stats 30d.

use crate::stats::RequestRecord;
use parking_lot::Mutex;
use rusqlite::{Connection, params};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

const FLUSH_INTERVAL_MS: u64 = 5000;
const RAW_RETENTION_HOURS: u64 = 24;
const HOURLY_RETENTION_DAYS: u64 = 30;
const CHANNEL_SIZE: usize = 4096;

pub struct StatsDB {
    sender: mpsc::Sender<RequestRecord>,
    conn: Arc<Mutex<Connection>>,
}

impl StatsDB {
    pub fn new() -> Self {
        let path = Path::new(".data");
        let _ = std::fs::create_dir_all(path);
        let conn = Connection::open(path.join("stats.db")).expect("open stats.db");
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS requests (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 ts_ms INTEGER NOT NULL,
                 duration_ms INTEGER NOT NULL,
                 status INTEGER NOT NULL,
                 model TEXT NOT NULL,
                 pipeline TEXT NOT NULL,
                 key_name TEXT NOT NULL,
                 tokens_in INTEGER NOT NULL,
                 tokens_out INTEGER NOT NULL,
                 cached INTEGER NOT NULL,
                 error TEXT
             );
             CREATE INDEX IF NOT EXISTS idx_requests_ts ON requests(ts_ms);
             CREATE INDEX IF NOT EXISTS idx_requests_model ON requests(model);
             CREATE INDEX IF NOT EXISTS idx_requests_key ON requests(key_name);
             CREATE TABLE IF NOT EXISTS hourly_stats (
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
             CREATE INDEX IF NOT EXISTS idx_hourly_ts ON hourly_stats(ts_hour);",
        )
        .expect("init stats schema");

        let conn = Arc::new(Mutex::new(conn));
        let (sender, mut receiver) = mpsc::channel::<RequestRecord>(CHANNEL_SIZE);
        let flush_conn = conn.clone();

        tokio::spawn(async move {
            let mut batch: Vec<RequestRecord> = Vec::with_capacity(128);
            loop {
                let timeout = tokio::time::sleep(Duration::from_millis(FLUSH_INTERVAL_MS));
                tokio::pin!(timeout);

                let should_flush = loop {
                    tokio::select! {
                        Some(rec) = receiver.recv() => {
                            batch.push(rec);
                            if batch.len() >= 128 {
                                break true;
                            }
                        }
                        _ = &mut timeout => break true,
                    }
                };

                if should_flush && !batch.is_empty() {
                    let to_insert = std::mem::take(&mut batch);
                    let conn = flush_conn.clone();
                    if let Err(e) = tokio::task::spawn_blocking(move || flush_batch(&conn, &to_insert)).await {
                        tracing::warn!("stats flush task failed: {}", e);
                    }
                    batch = Vec::with_capacity(128);
                }

                // Periodic cleanup
                let now = now_hour();
                if now % 6 == 0 {
                    // every ~6 hours, cheap guard
                    let conn = flush_conn.clone();
                    let _ = tokio::task::spawn_blocking(move || cleanup(&conn)).await;
                }
            }
        });

        Self { sender, conn }
    }

    pub fn sender(&self) -> mpsc::Sender<RequestRecord> {
        self.sender.clone()
    }

    /// Query historical buckets from SQLite. `bucket_ms` is the aggregation
    /// interval in milliseconds. Returns oldest → newest.
    pub fn query_buckets(
        &self,
        window_ms: u64,
        bucket_ms: u64,
    ) -> Result<Vec<crate::stats::StatsBucket>, rusqlite::Error> {
        let now_ms = now_millis();
        let start_ms = now_ms.saturating_sub(window_ms);
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT
                (ts_ms / ?) * ? as ts,
                COUNT(*) as count,
                SUM(CASE WHEN status >= 400 THEN 1 ELSE 0 END) as errors,
                SUM(CASE WHEN status IN (429, 503) THEN 1 ELSE 0 END) as throttled,
                AVG(duration_ms) as avg_latency,
                MAX(duration_ms) as max_latency,
                SUM(tokens_in) as tokens_in,
                SUM(tokens_out) as tokens_out,
                SUM(CASE WHEN cached = 1 THEN 1 ELSE 0 END) as cached
             FROM requests
             WHERE ts_ms >= ?
             GROUP BY ts
             ORDER BY ts ASC",
        )?;

        let mut rows = stmt.query(params![bucket_ms, bucket_ms, start_ms])?;
        let mut buckets = Vec::new();
        while let Some(row) = rows.next()? {
            buckets.push(crate::stats::StatsBucket {
                ts_ms: row.get(0)?,
                count: row.get(1)?,
                errors: row.get(2)?,
                throttled: row.get(3)?,
                avg_latency_ms: row.get::<_, Option<f64>>(4)?.unwrap_or(0.0),
                p50_latency_ms: 0.0,
                p95_latency_ms: 0.0,
                max_latency_ms: row.get::<_, Option<u64>>(5)?.unwrap_or(0) as u64,
                tokens_in: row.get::<_, Option<u64>>(6)?.unwrap_or(0) as u64,
                tokens_out: row.get::<_, Option<u64>>(7)?.unwrap_or(0) as u64,
                cached: row.get::<_, Option<u64>>(8)?.unwrap_or(0) as u64,
                by_model: std::collections::BTreeMap::new(),
            });
        }
        Ok(buckets)
    }

    pub fn query_summary(&self, window_ms: u64) -> Result<crate::stats::StatsSummary, rusqlite::Error> {
        let now_ms = now_millis();
        let start_ms = now_ms.saturating_sub(window_ms);
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT
                COUNT(*) as count,
                SUM(CASE WHEN status >= 400 THEN 1 ELSE 0 END) as errors,
                SUM(CASE WHEN status IN (429, 503) THEN 1 ELSE 0 END) as throttled,
                SUM(CASE WHEN cached = 1 THEN 1 ELSE 0 END) as cached,
                SUM(tokens_in) as tokens_in,
                SUM(tokens_out) as tokens_out,
                AVG(duration_ms) as avg_latency
             FROM requests
             WHERE ts_ms >= ?",
        )?;

        let row = stmt.query_row(params![start_ms], |row| {
            Ok(crate::stats::StatsSummary {
                count: row.get::<_, Option<u64>>(0)?.unwrap_or(0) as u64,
                errors: row.get::<_, Option<u64>>(1)?.unwrap_or(0) as u64,
                throttled: row.get::<_, Option<u64>>(2)?.unwrap_or(0) as u64,
                cached: row.get::<_, Option<u64>>(3)?.unwrap_or(0) as u64,
                tokens_in: row.get::<_, Option<u64>>(4)?.unwrap_or(0) as u64,
                tokens_out: row.get::<_, Option<u64>>(5)?.unwrap_or(0) as u64,
                avg_latency_ms: row.get::<_, Option<f64>>(6)?.unwrap_or(0.0),
            })
        })?;
        Ok(row)
    }

    /// Per-token/key stats for the dashboard.
    pub fn query_token_stats(
        &self,
        window_ms: u64,
    ) -> Result<Vec<TokenSummary>, rusqlite::Error> {
        let now_ms = now_millis();
        let start_ms = now_ms.saturating_sub(window_ms);
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT
                key_name,
                COUNT(*) as count,
                SUM(CASE WHEN status >= 400 THEN 1 ELSE 0 END) as errors,
                SUM(tokens_in) as tokens_in,
                SUM(tokens_out) as tokens_out,
                AVG(duration_ms) as avg_latency
             FROM requests
             WHERE ts_ms >= ?
             GROUP BY key_name
             ORDER BY count DESC",
        )?;

        let mut rows = stmt.query(params![start_ms])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(TokenSummary {
                key_name: row.get(0)?,
                count: row.get::<_, u64>(1)?,
                errors: row.get::<_, u64>(2)?,
                tokens_in: row.get::<_, u64>(3)?,
                tokens_out: row.get::<_, u64>(4)?,
                avg_latency_ms: row.get::<_, Option<f64>>(5)?.unwrap_or(0.0),
            });
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TokenSummary {
    pub key_name: String,
    pub count: u64,
    pub errors: u64,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub avg_latency_ms: f64,
}

fn flush_batch(conn: &Arc<Mutex<Connection>>, batch: &[RequestRecord]) -> Result<(), rusqlite::Error> {
    let mut conn = conn.lock();
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare_cached(
            "INSERT INTO requests
             (ts_ms, duration_ms, status, model, pipeline, key_name, tokens_in, tokens_out, cached, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )?;
        for rec in batch {
            stmt.execute(params![
                rec.ts_ms,
                rec.duration_ms,
                rec.status,
                rec.model,
                rec.pipeline,
                rec.key_name,
                rec.tokens_in,
                rec.tokens_out,
                rec.cached as i64,
                rec.error.as_deref(),
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

fn cleanup(conn: &Arc<Mutex<Connection>>) -> Result<(), rusqlite::Error> {
    let mut conn = conn.lock();
    let raw_cutoff = now_millis() - RAW_RETENTION_HOURS * 60 * 60 * 1000;
    let hourly_cutoff = now_hour() - (HOURLY_RETENTION_DAYS * 24) as u64;
    conn.execute("DELETE FROM requests WHERE ts_ms < ?", params![raw_cutoff])?;
    conn.execute(
        "DELETE FROM hourly_stats WHERE ts_hour < ?",
        params![hourly_cutoff],
    )?;
    Ok(())
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn now_hour() -> u64 {
    now_millis() / (60 * 60 * 1000)
}
