use crate::config::ConfigStore;
use std::sync::Arc;
use tokio::sync::Notify;

// ── Debounced config persistence (spec §2.8) ────────────────────────────────

pub struct DebouncedSaver {
    notify: Arc<Notify>,
}

impl DebouncedSaver {
    pub fn new(store: Arc<ConfigStore>) -> (Self, tokio::task::JoinHandle<()>) {
        let notify = Arc::new(Notify::new());
        let notify_clone = notify.clone();

        let handle = tokio::spawn(async move {
            loop {
                // Wait for a dirty ping
                notify_clone.notified().await;

                // Debounce: coalesce pings for 500ms
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;

                // Drain any additional pings that arrived during the debounce window
                // by using try_wait pattern
                loop {
                    let timeout = tokio::time::sleep(std::time::Duration::from_millis(500));
                    tokio::pin!(timeout);
                    tokio::select! {
                        _ = notify_clone.notified() => continue,
                        _ = &mut timeout => break,
                    }
                }

                // Write the current snapshot
                let cfg = store.load();
                match cfg.save_to_file() {
                    Ok(()) => tracing::info!("config saved to disk"),
                    Err(e) => tracing::warn!("config save failed: {}", e),
                }
            }
        });

        (Self { notify }, handle)
    }

    /// Signal that config has changed and should be persisted.
    pub fn ping(&self) {
        self.notify.notify_one();
    }
}
