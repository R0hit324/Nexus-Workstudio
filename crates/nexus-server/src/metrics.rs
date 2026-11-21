use std::sync::atomic::{AtomicUsize, Ordering};

/// Lightweight, lock-free server observability counters.
///
/// These feed the public [`crate::ServerState`] and future Prometheus-style
/// exporters. Ordering is always `Relaxed`; counters are diagnostics, not
/// synchronization primitives.
#[derive(Debug, Default)]
pub struct Metrics {
    /// Cubes currently loaded in the registry.
    pub active_cubes: AtomicUsize,
    /// Requests dispatched since process start.
    pub requests_total: AtomicUsize,
    /// Events published to the event bus.
    pub events_total: AtomicUsize,
    /// Events dropped because the destination outbound queue was full.
    pub events_dropped: AtomicUsize,
    /// Errors returned to clients.
    pub errors_total: AtomicUsize,
    /// Protocol messages decoded from the network.
    pub messages_in: AtomicUsize,
    /// Protocol messages sent to clients.
    pub messages_out: AtomicUsize,
    /// Network bytes read (sum of message sizes before reply).
    pub network_bytes_in: AtomicUsize,
    /// Network bytes written (sum of message sizes).
    pub network_bytes_out: AtomicUsize,
    /// Workspace filesystem operations.
    pub workspace_operations: AtomicUsize,
    /// Authentication attempts (successful or not).
    pub authentication_attempts: AtomicUsize,
}

impl Metrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn request_started(&self) {
        self.requests_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn event_published(&self) {
        self.events_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn event_dropped(&self) {
        self.events_dropped.fetch_add(1, Ordering::Relaxed);
    }

    pub fn message_in(&self) {
        self.messages_in.fetch_add(1, Ordering::Relaxed);
    }

    pub fn message_out(&self) {
        self.messages_out.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_network_in(&self, bytes: usize) {
        self.network_bytes_in.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn record_network_out(&self, bytes: usize) {
        self.network_bytes_out.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn workspace_operation(&self) {
        self.workspace_operations.fetch_add(1, Ordering::Relaxed);
    }

    pub fn authentication_attempt(&self) {
        self.authentication_attempts.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn counters_increment() {
        let metrics = Metrics::new();
        assert_eq!(metrics.requests_total.load(Ordering::Relaxed), 0);
        metrics.request_started();
        metrics.event_published();
        metrics.message_in();
        assert_eq!(metrics.requests_total.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.events_total.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.messages_in.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn counters_shared_across_arcs() {
        let metrics = Arc::new(Metrics::new());
        let a = metrics.clone();
        let b = metrics.clone();
        a.request_started();
        assert_eq!(b.requests_total.load(Ordering::Relaxed), 1);
    }
}
