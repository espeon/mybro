use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

// ── HandoffCache (spec §4) ───────────────────────────────────────────────────

struct CacheEntry {
    desc: std::sync::Arc<str>,
    inserted: Instant,
}

pub struct HandoffCache {
    inner: Mutex<lru::LruCache<[u8; 32], CacheEntry>>,
    ttl: AtomicU64, // millis
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
}

impl HandoffCache {
    pub fn new(max_size: usize, ttl: std::time::Duration) -> Self {
        Self {
            inner: Mutex::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(max_size).unwrap_or_else(|| {
                    std::num::NonZeroUsize::new(50).unwrap()
                }),
            )),
            ttl: AtomicU64::new(ttl.as_millis() as u64),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
        }
    }

    pub fn get(&self, digest: &[u8; 32]) -> Option<std::sync::Arc<str>> {
        let ttl_ms = self.ttl.load(Ordering::Relaxed);
        let ttl = std::time::Duration::from_millis(ttl_ms);
        let mut inner = self.inner.lock();
        if let Some(entry) = inner.get(digest) {
            if entry.inserted.elapsed() < ttl {
                self.hits.fetch_add(1, Ordering::Relaxed);
                return Some(entry.desc.clone());
            } else {
                // expired
                inner.pop(digest);
            }
        }
        self.misses.fetch_add(1, Ordering::Relaxed);
        None
    }

    pub fn set(&self, digest: [u8; 32], desc: std::sync::Arc<str>) {
        let mut inner = self.inner.lock();
        if inner.len() >= inner.cap().get() {
            if inner.pop_lru().is_some() {
                self.evictions.fetch_add(1, Ordering::Relaxed);
            }
        }
        inner.put(digest, CacheEntry { desc, inserted: Instant::now() });
    }

    pub fn stats(&self) -> HandoffCacheStats {
        let inner = self.inner.lock();
        HandoffCacheStats {
            size: inner.len(),
            max_size: inner.cap().get(),
            ttl_ms: self.ttl.load(Ordering::Relaxed),
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
        }
    }

    pub fn resize(&self, max_size: usize, ttl: std::time::Duration) {
        let mut inner = self.inner.lock();
        inner.resize(
            std::num::NonZeroUsize::new(max_size)
                .unwrap_or_else(|| std::num::NonZeroUsize::new(50).unwrap()),
        );
        self.ttl.store(ttl.as_millis() as u64, Ordering::Relaxed);
    }
}

#[derive(Debug, serde::Serialize)]
pub struct HandoffCacheStats {
    pub size: usize,
    #[serde(rename = "maxSize")]
    pub max_size: usize,
    #[serde(rename = "ttlMs")]
    pub ttl_ms: u64,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
}
