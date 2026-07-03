use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Semaphore;

// ── Gate (spec §5) — semaphore-based concurrency gating ─────────────────────────

/// Default active-connection limit when no upstream limit is available and no
/// override is configured. This is a safe conservative default (UMANS API often
/// reports limits around 4-8).
pub const DEFAULT_CONCURRENCY_LIMIT: usize = 4;

pub struct Gate {
    sem: Arc<Semaphore>,
    /// Permits currently issued to the semaphore (the configured limit).
    granted: AtomicUsize,
    /// In-flight proxied requests.
    active: AtomicUsize,
    /// Tasks waiting on the semaphore.
    queued: AtomicUsize,
    /// Queue-full 503 count.
    throttled: AtomicU64,
}

/// RAII guard — increments `active` on creation, decrements on drop.
pub struct ActiveGuard {
    gate: Arc<Gate>,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.gate.active.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Gate {
    /// Create a new gate. `limit = None` uses the safe `DEFAULT_CONCURRENCY_LIMIT`.
    pub fn new(limit: Option<usize>) -> Arc<Self> {
        let initial = limit.unwrap_or(DEFAULT_CONCURRENCY_LIMIT);
        let sem = Arc::new(Semaphore::new(initial));
        Arc::new(Self {
            sem,
            granted: AtomicUsize::new(initial),
            active: AtomicUsize::new(0),
            queued: AtomicUsize::new(0),
            throttled: AtomicU64::new(0),
        })
    }

    /// Attempt admission. Returns `Ok(guard)` if admitted, or `Err(())` if the
    /// queue is full (caller should respond 503 queue_full).
    pub async fn acquire(self: &Arc<Self>) -> Result<ActiveGuard, ()> {
        let queued_now = self.queued.load(Ordering::Relaxed);
        if queued_now >= crate::constants::MAX_QUEUE_SIZE {
            // Check if a permit is immediately available
            match self.sem.clone().try_acquire_owned() {
                Ok(permit) => {
                    self.queued.fetch_sub(1, Ordering::Relaxed);
                    self.active.fetch_add(1, Ordering::Relaxed);
                    return Ok(ActiveGuard {
                        gate: self.clone(),
                        _permit: permit,
                    });
                }
                Err(_) => {
                    self.throttled.fetch_add(1, Ordering::Relaxed);
                    return Err(());
                }
            }
        }

        self.queued.fetch_add(1, Ordering::Relaxed);
        let permit = self
            .sem
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore closed");
        self.queued.fetch_sub(1, Ordering::Relaxed);
        self.active.fetch_add(1, Ordering::Relaxed);
        Ok(ActiveGuard {
            gate: self.clone(),
            _permit: permit,
        })
    }

    pub fn active(&self) -> usize {
        self.active.load(Ordering::Relaxed)
    }

    pub fn queued(&self) -> usize {
        self.queued.load(Ordering::Relaxed)
    }

    pub fn throttled(&self) -> u64 {
        self.throttled.load(Ordering::Relaxed)
    }

    pub fn bump_throttled(&self) {
        self.throttled.fetch_add(1, Ordering::Relaxed);
    }

    /// Resize the semaphore when the effective concurrency limit changes.
    /// `None` means "use the safe default" (not unbounded).
    pub fn reconcile(self: &Arc<Self>, new_limit: Option<usize>) {
        let new = new_limit.unwrap_or(DEFAULT_CONCURRENCY_LIMIT);
        let current = self.granted.load(Ordering::Relaxed);
        if new > current {
            self.sem.add_permits(new - current);
            self.granted.store(new, Ordering::Relaxed);
        } else if new < current {
            // Shrink: acquire and forget the difference.
            let diff = current - new;
            for _ in 0..diff {
                if let Ok(p) = self.sem.clone().try_acquire_owned() {
                    p.forget();
                }
            }
            self.granted.store(new, Ordering::Relaxed);
        }
    }
}
