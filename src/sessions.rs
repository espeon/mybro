use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

// ── Session tracking (spec §12) ──────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
struct Session {
    token_index: usize,
    request_count: u64,
    sess_num: u64,
}

pub struct ConvMap {
    inner: Mutex<lru::LruCache<String, Session>>,
    counter: AtomicU64,
}

/// Result of touching a fingerprint — whether it was a new session.
pub struct TouchResult {
    pub sess_num: u64,
    pub request_count: u64,
    pub token_index: Option<usize>,
    pub is_new: bool,
}

const EVICT_THRESHOLD: usize = 8_000;

impl ConvMap {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(crate::constants::CONVERSATION_MAP_MAX).unwrap(),
            )),
            counter: AtomicU64::new(0),
        }
    }

    /// Look up a fingerprint without creating/updating (spec §12.2 touch).
    pub fn touch(&self, fp: &str) -> Option<TouchResult> {
        let mut inner = self.inner.lock();
        if let Some(s) = inner.get(fp) {
            return Some(TouchResult {
                sess_num: s.sess_num,
                request_count: s.request_count,
                token_index: Some(s.token_index),
                is_new: false,
            });
        }
        None
    }

    /// Create or update a session entry (spec §12.2 track).
    pub fn track(&self, fp: &str, token_index: usize) -> TouchResult {
        let mut inner = self.inner.lock();
        if let Some(s) = inner.get_mut(fp) {
            s.request_count += 1;
            s.token_index = token_index;
            TouchResult {
                sess_num: s.sess_num,
                request_count: s.request_count,
                token_index: Some(s.token_index),
                is_new: false,
            }
        } else {
            // Evict down to 80% if at capacity
            if inner.len() >= crate::constants::CONVERSATION_MAP_MAX {
                while inner.len() > EVICT_THRESHOLD {
                    inner.pop_lru();
                }
            }
            let sess_num = self.counter.fetch_add(1, Ordering::Relaxed) + 1;
            let s = Session {
                token_index,
                request_count: 1,
                sess_num,
            };
            inner.put(fp.to_string(), s);
            TouchResult {
                sess_num,
                request_count: 1,
                token_index: Some(token_index),
                is_new: true,
            }
        }
    }
}

impl Default for ConvMap {
    fn default() -> Self {
        Self::new()
    }
}
