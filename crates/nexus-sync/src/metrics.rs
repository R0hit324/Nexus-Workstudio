use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Synchronization metrics for monitoring and observability.
#[derive(Debug, Default)]
pub struct SyncMetrics {
    pub operations_sent: AtomicU64,
    pub operations_received: AtomicU64,
    pub operations_accepted: AtomicU64,
    pub operations_rejected: AtomicU64,
    pub operations_transformed: AtomicU64,
    pub pending_operations: AtomicU64,
    pub sync_latency_ms: AtomicU64,
    pub reconciliation_latency_ms: AtomicU64,
    pub missing_event_requests: AtomicU64,
    pub desynchronization_count: AtomicU64,
    pub cursor_updates_sent: AtomicU64,
    pub cursor_updates_received: AtomicU64,
    pub presence_updates_sent: AtomicU64,
    pub presence_updates_received: AtomicU64,
    pub file_opens: AtomicU64,
    pub file_creates: AtomicU64,
    pub file_deletes: AtomicU64,
    pub file_renames: AtomicU64,
}

impl SyncMetrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn record_op_sent(&self) {
        self.operations_sent.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_op_received(&self) {
        self.operations_received.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_op_accepted(&self) {
        self.operations_accepted.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_op_rejected(&self) {
        self.operations_rejected.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_op_transformed(&self) {
        self.operations_transformed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn set_pending_ops(&self, count: u64) {
        self.pending_operations.store(count, Ordering::Relaxed);
    }

    pub fn record_sync_latency(&self, ms: u64) {
        self.sync_latency_ms.store(ms, Ordering::Relaxed);
    }

    pub fn record_reconciliation_latency(&self, ms: u64) {
        self.reconciliation_latency_ms.store(ms, Ordering::Relaxed);
    }

    pub fn record_missing_event_request(&self) {
        self.missing_event_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_desynchronization(&self) {
        self.desynchronization_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_cursor_sent(&self) {
        self.cursor_updates_sent.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_cursor_received(&self) {
        self.cursor_updates_received.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_presence_sent(&self) {
        self.presence_updates_sent.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_presence_received(&self) {
        self.presence_updates_received
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_file_open(&self) {
        self.file_opens.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_file_create(&self) {
        self.file_creates.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_file_delete(&self) {
        self.file_deletes.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_file_rename(&self) {
        self.file_renames.fetch_add(1, Ordering::Relaxed);
    }

    /// Get a snapshot of all metrics.
    pub fn snapshot(&self) -> SyncMetricsSnapshot {
        SyncMetricsSnapshot {
            operations_sent: self.operations_sent.load(Ordering::Relaxed),
            operations_received: self.operations_received.load(Ordering::Relaxed),
            operations_accepted: self.operations_accepted.load(Ordering::Relaxed),
            operations_rejected: self.operations_rejected.load(Ordering::Relaxed),
            operations_transformed: self.operations_transformed.load(Ordering::Relaxed),
            pending_operations: self.pending_operations.load(Ordering::Relaxed),
            sync_latency_ms: self.sync_latency_ms.load(Ordering::Relaxed),
            reconciliation_latency_ms: self.reconciliation_latency_ms.load(Ordering::Relaxed),
            missing_event_requests: self.missing_event_requests.load(Ordering::Relaxed),
            desynchronization_count: self.desynchronization_count.load(Ordering::Relaxed),
            cursor_updates_sent: self.cursor_updates_sent.load(Ordering::Relaxed),
            cursor_updates_received: self.cursor_updates_received.load(Ordering::Relaxed),
            presence_updates_sent: self.presence_updates_sent.load(Ordering::Relaxed),
            presence_updates_received: self.presence_updates_received.load(Ordering::Relaxed),
            file_opens: self.file_opens.load(Ordering::Relaxed),
            file_creates: self.file_creates.load(Ordering::Relaxed),
            file_deletes: self.file_deletes.load(Ordering::Relaxed),
            file_renames: self.file_renames.load(Ordering::Relaxed),
        }
    }
}

/// Immutable snapshot of synchronization metrics.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SyncMetricsSnapshot {
    pub operations_sent: u64,
    pub operations_received: u64,
    pub operations_accepted: u64,
    pub operations_rejected: u64,
    pub operations_transformed: u64,
    pub pending_operations: u64,
    pub sync_latency_ms: u64,
    pub reconciliation_latency_ms: u64,
    pub missing_event_requests: u64,
    pub desynchronization_count: u64,
    pub cursor_updates_sent: u64,
    pub cursor_updates_received: u64,
    pub presence_updates_sent: u64,
    pub presence_updates_received: u64,
    pub file_opens: u64,
    pub file_creates: u64,
    pub file_deletes: u64,
    pub file_renames: u64,
}
