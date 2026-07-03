use crate::config::ConfigKey;
use parking_lot::Mutex;
use serde::Serialize;
use std::sync::Arc;
use std::time::{Duration, Instant};

// ── KeyEntry (spec §3.1) ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct KeyEntry {
    pub key: String,
    pub name: String,
    pub healthy: bool,
    pub last_error: Option<Instant>,
    pub cooldown: Duration,
}

impl KeyEntry {
    pub fn from_config(ck: &ConfigKey) -> Self {
        Self {
            key: ck.key.clone(),
            name: ck.name.clone(),
            healthy: true,
            last_error: None,
            cooldown: DEFAULT_COOLDOWN,
        }
    }

    fn is_usable(&self) -> bool {
        self.key.is_empty().not() && (self.healthy || self.is_past_cooldown())
    }

    fn is_past_cooldown(&self) -> bool {
        match self.last_error {
            None => true,
            Some(t) => t.elapsed() >= self.cooldown,
        }
    }
}

const DEFAULT_COOLDOWN: Duration = Duration::from_secs(30);

// ── KeySlot — detached clone handed to callers (no lock held during call) ─────

#[derive(Debug, Clone)]
pub struct KeySlot {
    pub index: usize,
    pub key: Arc<str>,
    pub name: Arc<str>,
}

// ── KeyPool (spec §3) ────────────────────────────────────────────────────────

#[derive(Debug)]
struct PoolInner {
    entries: Vec<KeyEntry>,
    cursor: usize,
}

pub struct KeyPool {
    inner: Mutex<PoolInner>,
}

impl KeyPool {
    pub fn new(entries: Vec<KeyEntry>) -> Self {
        Self {
            inner: Mutex::new(PoolInner {
                entries,
                cursor: 0,
            }),
        }
    }

    /// Acquire a key. If `preferred` is valid and usable, use it; otherwise round-robin.
    /// Returns `None` if no usable keys.
    pub fn acquire(&self, preferred: Option<usize>) -> Option<KeySlot> {
        let mut inner = self.inner.lock();
        let n = inner.entries.len();
        if n == 0 {
            return None;
        }

        // Try preferred first
        if let Some(idx) = preferred {
            if idx < n {
                let e = &mut inner.entries[idx];
                if e.is_usable() {
                    e.healthy = true;
                    e.last_error = None;
                    let key = Arc::from(e.key.as_str());
                    let name = Arc::from(e.name.as_str());
                    return Some(KeySlot {
                        index: idx,
                        key,
                        name,
                    });
                }
            }
        }

        // Round-robin from cursor
        for i in 0..n {
            let idx = (inner.cursor + i) % n;
            let e = &mut inner.entries[idx];
            if e.is_usable() {
                e.healthy = true;
                e.last_error = None;
                let key = Arc::from(e.key.as_str());
                let name = Arc::from(e.name.as_str());
                inner.cursor = (idx + 1) % n;
                return Some(KeySlot {
                    index: idx,
                    key,
                    name,
                });
            }
        }
        None
    }

    pub fn mark_unhealthy(&self, index: usize, status: u16) {
        let mut inner = self.inner.lock();
        if index < inner.entries.len() {
            let e = &mut inner.entries[index];
            e.healthy = false;
            e.last_error = Some(Instant::now());
            e.cooldown = cooldown_for_status(status);
        }
    }

    pub fn mark_healthy(&self, index: usize) {
        let mut inner = self.inner.lock();
        if index < inner.entries.len() {
            let e = &mut inner.entries[index];
            e.healthy = true;
            e.last_error = None;
        }
    }

    pub fn healthy_count(&self) -> usize {
        let inner = self.inner.lock();
        inner.entries.iter().filter(|e| e.is_usable()).count()
    }

    pub fn total(&self) -> usize {
        let inner = self.inner.lock();
        inner.entries.len()
    }

    /// Replace pool entries in one locked swap (spec §3.8).
    pub fn rebuild(&self, entries: Vec<KeyEntry>) {
        let mut inner = self.inner.lock();
        inner.entries = entries;
        inner.cursor = 0;
    }

    /// Snapshot for /healthz (spec §3.7)
    pub fn state(&self) -> Vec<KeyState> {
        let inner = self.inner.lock();
        inner
            .entries
            .iter()
            .map(|e| {
                let (status, remaining_ms) = if e.key.is_empty() {
                    ("none", 0u64)
                } else if e.healthy || e.is_past_cooldown() {
                    ("active", 0u64)
                } else {
                    let rem = e
                        .last_error
                        .map(|t| {
                            let elapsed = t.elapsed();
                            if elapsed >= e.cooldown {
                                0
                            } else {
                                (e.cooldown - elapsed).as_millis() as u64
                            }
                        })
                        .unwrap_or(0);
                    ("cooldown", rem)
                };
                KeyState {
                    name: e.name.clone(),
                    status,
                    healthy: e.healthy,
                    remaining_cooldown_ms: remaining_ms,
                    token: crate::config::mask_token(&e.key),
                }
            })
            .collect()
    }
}

fn cooldown_for_status(status: u16) -> Duration {
    if status >= 503 {
        Duration::from_secs(60)
    } else if status >= 502 {
        Duration::from_secs(30)
    } else {
        Duration::from_secs(10)
    }
}

#[derive(Debug, Serialize)]
pub struct KeyState {
    pub name: String,
    pub status: &'static str,
    pub healthy: bool,
    #[serde(rename = "remainingCooldown")]
    pub remaining_cooldown_ms: u64,
    pub token: String,
}

// ── tiny helper trait to avoid pulling in `bool::then` nightly issues ─────────

trait Not {
    fn not(self) -> bool;
}

impl Not for bool {
    fn not(self) -> bool {
        !self
    }
}
