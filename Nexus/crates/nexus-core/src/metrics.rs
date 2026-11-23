//! Central performance metrics for Nexus (Part 13).
//!
//! Cheap, lock-free accumulators for the performance signals that Part 13
//! tracks: latency, CPU, memory, network, and throughput. Counters live in
//! atomics so hot paths can record without taking a lock; [`PerformanceSnapshot`]
//! is an immutable point-in-time view used for dashboards, logs, and tests.
//!
//! Optimization policy (spec Part 13, section 1): measure first, profile
//! bottlenecks, optimize, benchmark again, and keep every optimization simple,
//! measurable, and predictable.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

/// Lock-free performance accumulators.
///
/// All counters are updated through the [`PerformanceMetrics::record_*`]
/// helpers, which only touch atomics, so recording is safe from async tasks,
/// render loops, and input handlers without synchronization overhead.
#[derive(Debug, Default)]
pub struct PerformanceMetrics {
    /// Sum of observed operation latencies in milliseconds.
    pub latency_ms_sum: AtomicU64,
    /// Number of latency observations recorded.
    pub latency_ms_count: AtomicU64,
    /// Sum of observed CPU time in milliseconds.
    pub cpu_ms_sum: AtomicU64,
    /// Number of CPU observations recorded.
    pub cpu_ms_count: AtomicU64,
    /// Cumulative bytes allocated by the process.
    pub memory_bytes: AtomicU64,
    /// Cumulative network bytes sent.
    pub network_bytes_sent: AtomicU64,
    /// Cumulative network bytes received.
    pub network_bytes_received: AtomicU64,
    /// Cumulative network messages sent.
    pub network_messages_sent: AtomicU64,
    /// Cumulative network messages received.
    pub network_messages_received: AtomicU64,
    /// Total operations performed (throughput counter).
    pub operations: AtomicU64,
}

/// Immutable point-in-time performance snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PerformanceSnapshot {
    /// Sum of observed latencies, in milliseconds.
    pub latency_ms_sum: u64,
    /// Number of latency observations.
    pub latency_ms_count: u64,
    /// Average observed latency, in milliseconds.
    pub latency_ms_avg: f64,
    /// Sum of observed CPU time, in milliseconds.
    pub cpu_ms_sum: u64,
    /// Number of CPU observations.
    pub cpu_ms_count: u64,
    /// Average observed CPU time, in milliseconds.
    pub cpu_ms_avg: f64,
    /// Cumulative allocated bytes.
    pub memory_bytes: u64,
    /// Cumulative network bytes sent.
    pub network_bytes_sent: u64,
    /// Cumulative network bytes received.
    pub network_bytes_received: u64,
    /// Cumulative network messages sent.
    pub network_messages_sent: u64,
    /// Cumulative network messages received.
    pub network_messages_received: u64,
    /// Total operations performed.
    pub operations: u64,
}

impl PerformanceMetrics {
    /// Creates a new set of performance accumulators.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an operation latency in milliseconds.
    pub fn record_latency_ms(&self, ms: u64) {
        self.latency_ms_sum.fetch_add(ms, Ordering::Relaxed);
        self.latency_ms_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Records observed CPU time in milliseconds.
    pub fn record_cpu_ms(&self, ms: u64) {
        self.cpu_ms_sum.fetch_add(ms, Ordering::Relaxed);
        self.cpu_ms_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a memory allocation in bytes.
    pub fn record_memory_allocation(&self, bytes: u64) {
        self.memory_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Records network bytes sent.
    pub fn record_network_bytes_sent(&self, bytes: u64) {
        self.network_bytes_sent.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Records network bytes received.
    pub fn record_network_bytes_received(&self, bytes: u64) {
        self.network_bytes_received
            .fetch_add(bytes, Ordering::Relaxed);
    }

    /// Records a sent network message.
    pub fn record_network_message_sent(&self) {
        self.network_messages_sent.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a received network message.
    pub fn record_network_message_received(&self) {
        self.network_messages_received
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Records an operation for throughput accounting.
    pub fn record_operation(&self) {
        self.operations.fetch_add(1, Ordering::Relaxed);
    }

    /// A point-in-time snapshot of all performance counters.
    #[must_use]
    pub fn snapshot(&self) -> PerformanceSnapshot {
        let latency_ms_sum = self.latency_ms_sum.load(Ordering::Relaxed);
        let latency_ms_count = self.latency_ms_count.load(Ordering::Relaxed);
        let cpu_ms_sum = self.cpu_ms_sum.load(Ordering::Relaxed);
        let cpu_ms_count = self.cpu_ms_count.load(Ordering::Relaxed);
        PerformanceSnapshot {
            latency_ms_sum,
            latency_ms_count,
            latency_ms_avg: average_ms(latency_ms_sum, latency_ms_count),
            cpu_ms_sum,
            cpu_ms_count,
            cpu_ms_avg: average_ms(cpu_ms_sum, cpu_ms_count),
            memory_bytes: self.memory_bytes.load(Ordering::Relaxed),
            network_bytes_sent: self.network_bytes_sent.load(Ordering::Relaxed),
            network_bytes_received: self.network_bytes_received.load(Ordering::Relaxed),
            network_messages_sent: self.network_messages_sent.load(Ordering::Relaxed),
            network_messages_received: self.network_messages_received.load(Ordering::Relaxed),
            operations: self.operations.load(Ordering::Relaxed),
        }
    }
}

impl fmt::Display for PerformanceSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "latency_ms_sum={} latency_ms_count={} latency_ms_avg={:.3} cpu_ms_sum={} cpu_ms_count={} cpu_ms_avg={:.3} memory_bytes={} network_bytes_sent={} network_bytes_received={} network_messages_sent={} network_messages_received={} operations={}",
            self.latency_ms_sum,
            self.latency_ms_count,
            self.latency_ms_avg,
            self.cpu_ms_sum,
            self.cpu_ms_count,
            self.cpu_ms_avg,
            self.memory_bytes,
            self.network_bytes_sent,
            self.network_bytes_received,
            self.network_messages_sent,
            self.network_messages_received,
            self.operations
        )
    }
}

/// Averages `sum / count` in milliseconds, or zero when no samples exist.
fn average_ms(sum: u64, count: u64) -> f64 {
    if count == 0 {
        return 0.0;
    }
    sum as f64 / count as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_snapshot_reports_zeroed_counters() {
        let metrics = PerformanceMetrics::new();
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.latency_ms_sum, 0);
        assert_eq!(snapshot.latency_ms_count, 0);
        assert_eq!(snapshot.cpu_ms_sum, 0);
        assert_eq!(snapshot.memory_bytes, 0);
        assert_eq!(snapshot.operations, 0);
        assert!(!format!("{snapshot}").is_empty());
    }

    #[test]
    fn latency_is_rolled_up_into_average() {
        let metrics = PerformanceMetrics::new();
        metrics.record_latency_ms(3);
        metrics.record_latency_ms(5);
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.latency_ms_sum, 8);
        assert_eq!(snapshot.latency_ms_count, 2);
        assert!((snapshot.latency_ms_avg - 4.0).abs() < 1e-9);
    }

    #[test]
    fn cpu_and_memory_counters_accumulate() {
        let metrics = PerformanceMetrics::new();
        metrics.record_cpu_ms(12);
        metrics.record_memory_allocation(1024);
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.cpu_ms_sum, 12);
        assert_eq!(snapshot.cpu_ms_count, 1);
        assert_eq!(snapshot.memory_bytes, 1024);
    }

    #[test]
    fn network_and_operation_counters_accumulate() {
        let metrics = PerformanceMetrics::new();
        metrics.record_network_bytes_sent(100);
        metrics.record_network_bytes_received(50);
        metrics.record_network_message_sent();
        metrics.record_network_message_received();
        metrics.record_operation();
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.network_bytes_sent, 100);
        assert_eq!(snapshot.network_bytes_received, 50);
        assert_eq!(snapshot.network_messages_sent, 1);
        assert_eq!(snapshot.network_messages_received, 1);
        assert_eq!(snapshot.operations, 1);
    }
}
