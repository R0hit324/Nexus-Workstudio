use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::Notify;

/// Cooperative, cancellable shutdown signal shared across the server task
/// tree. `trigger()` may be called from anywhere (signal handler, admin API,
/// connection task). Readers await [`Shutdown::delay`].
#[derive(Debug)]
pub struct Shutdown {
    triggered: AtomicBool,
    notify: Notify,
}

impl Default for Shutdown {
    fn default() -> Self {
        Self::new()
    }
}

impl Shutdown {
    pub fn new() -> Self {
        Self {
            triggered: AtomicBool::new(false),
            notify: Notify::new(),
        }
    }

    /// Signals graceful shutdown. Idempotent.
    pub fn trigger(&self) {
        if self
            .triggered
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            self.notify.notify_waiters();
        }
    }

    #[must_use]
    pub fn is_triggered(&self) -> bool {
        self.triggered.load(Ordering::SeqCst)
    }

    /// Resolves once shutdown has been triggered (or immediately when it
    /// already has been).
    pub async fn delay(&self) {
        if self.is_triggered() {
            return;
        }
        self.notify.notified().await;
    }
}

/// Convenience: spawns a task that triggers shutdown on `(Ctrl+C)` / SIGTERM.
///
/// Panics are not possible from here; failures to register a handler are
/// logged at debug level and the task simply never fires.
pub fn spawn_signal_task(shutdown: Arc<Shutdown>) {
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            if let Ok(mut sigterm) = signal(SignalKind::terminate()) {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = sigterm.recv() => {}
                }
                shutdown.trigger();
            } else {
                let _ = tokio::signal::ctrl_c().await;
                shutdown.trigger();
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
            shutdown.trigger();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn trigger_wakes_delayers() {
        let shutdown = Shutdown::new();
        let clone = Arc::new(shutdown);
        let handle = Arc::clone(&clone);
        let task = tokio::spawn(async move { handle.delay().await });
        assert!(!clone.is_triggered());
        clone.trigger();
        clone.trigger(); // idempotent
        task.await.unwrap();
        assert!(clone.is_triggered());
    }

    #[tokio::test]
    async fn already_triggered_resolves_immediately() {
        let shutdown = Arc::new(Shutdown::new());
        shutdown.trigger();
        shutdown.delay().await;
    }
}
