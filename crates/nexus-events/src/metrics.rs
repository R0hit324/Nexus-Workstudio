use std::sync::atomic::{AtomicU64, Ordering};

/// Live counters and gauges for the event engine (spec section 55).
///
/// Cheap to update during hot paths (atomics only, never locks) and cheap to
/// snapshot for dashboards, logs, or the server's metrics endpoint.
#[derive(Debug, Default)]
pub struct EventMetrics {
    pub published: AtomicU64,
    pub persisted: AtomicU64,
    pub failed: AtomicU64,
    pub replayed: AtomicU64,
    pub delivered: AtomicU64,
    pub dropped_transient: AtomicU64,
    pub slow_subscriber_drops: AtomicU64,
    pub subscribers: AtomicU64,
    /// Milliseconds published -> fully flushed to store (best effort bucket 0).
    pub persist_delay_bucket0: AtomicU64,
}

/// An immutable snapshot of [`EventMetrics`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MetricsSnapshot {
    pub published: u64,
    pub persisted: u64,
    pub failed: u64,
    pub replayed: u64,
    pub delivered: u64,
    pub dropped_transient: u64,
    pub slow_subscriber_drops: u64,
    pub subscribers: u64,
    pub persist_delay_bucket0: u64,
}

impl EventMetrics {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_published(&self) {
        self.published.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_persisted(&self) {
        self.persisted.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_failed(&self) {
        self.failed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_replayed(&self, n: usize) {
        self.replayed.fetch_add(n as u64, Ordering::Relaxed);
    }

    pub fn record_delivered(&self) {
        self.delivered.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_dropped_transient(&self) {
        self.dropped_transient.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_slow_subscriber_drop(&self) {
        self.slow_subscriber_drops.fetch_add(1, Ordering::Relaxed);
    }

    pub fn observe_subscribers(&self, count: u64) {
        self.subscribers.store(count, Ordering::Relaxed);
    }

    pub fn record_persist_delay_ms(&self, ms: u64) {
        self.persist_delay_bucket0.fetch_add(ms, Ordering::Relaxed);
    }

    /// A point-in-time snapshot for reporting.
    #[must_use]
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            published: self.published.load(Ordering::Relaxed),
            persisted: self.persisted.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            replayed: self.replayed.load(Ordering::Relaxed),
            delivered: self.delivered.load(Ordering::Relaxed),
            dropped_transient: self.dropped_transient.load(Ordering::Relaxed),
            slow_subscriber_drops: self.slow_subscriber_drops.load(Ordering::Relaxed),
            subscribers: self.subscribers.load(Ordering::Relaxed),
            persist_delay_bucket0: self.persist_delay_bucket0.load(Ordering::Relaxed),
        }
    }
}

impl std::fmt::Display for MetricsSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "published={} persisted={} failed={} replayed={} delivered={} drops(transient={},slow_sub={}) subscribers={}",
            self.published,
            self.persisted,
            self.failed,
            self.replayed,
            self.delivered,
            self.dropped_transient,
            self.slow_subscriber_drops,
            self.subscribers,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_count_and_snapshot() {
        let m = EventMetrics::new();
        m.record_published();
        m.record_published();
        m.record_persisted();
        m.record_failed();
        m.record_replayed(3);
        let snap = m.snapshot();
        assert_eq!(snap.published, 2);
        assert_eq!(snap.persisted, 1);
        assert_eq!(snap.failed, 1);
        assert_eq!(snap.replayed, 3);
        assert!(!format!("{snap}").is_empty());
    }
}
