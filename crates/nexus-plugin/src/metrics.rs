//! Per-plugin observability counters (spec sections 74–75).
//!
//! Cheap atomics, no locks on the hot paths: activation times, failure
//! counts, event/command throughput. Structured output is the host's job
//! (server admin surfaces, Part 13 dashboards).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::id::PluginId;

#[derive(Debug, Default)]
struct PerPlugin {
    activations: AtomicU64,
    failures: AtomicU64,
    events_published: AtomicU64,
    events_dropped: AtomicU64,
    commands_executed: AtomicU64,
    last_load_millis: AtomicU64,
    last_error: RwLock<Option<String>>,
}

/// Snapshot of one plugin's counters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginMetricsSnapshot {
    pub activations: u64,
    pub failures: u64,
    pub events_published: u64,
    pub events_dropped: u64,
    pub commands_executed: u64,
    pub last_load: Duration,
}

#[derive(Debug, Default)]
pub struct PluginMetrics {
    plugins: RwLock<BTreeMap<PluginId, Arc<PerPlugin>>>,
}

impl PluginMetrics {
    /// Records a successful activation with its wall time.
    pub fn record_activation(&self, plugin: &PluginId, elapsed: Duration) {
        let entry = self.entry(plugin);
        entry.activations.fetch_add(1, Ordering::Relaxed);
        entry
            .last_load_millis
            .store(elapsed.as_millis() as u64, Ordering::Relaxed);
    }

    /// Records one plugin failure with its last error line.
    pub fn record_failure(&self, plugin: &PluginId, error: &str) {
        let entry = self.entry(plugin);
        entry.failures.fetch_add(1, Ordering::Relaxed);
        *entry.last_error.write().expect("metrics error lock") = Some(String::from(error));
    }

    /// Records a published (accepted at the hub) plugin event.
    pub fn record_event_published(&self, plugin: &PluginId) {
        self.entry(plugin)
            .events_published
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Records a plugin event rejected/dropped (rate limit, validation).
    pub fn record_event_dropped(&self, plugin: &PluginId) {
        self.entry(plugin)
            .events_dropped
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Records a completed command execution.
    pub fn record_command_executed(&self, plugin: &PluginId) {
        self.entry(plugin)
            .commands_executed
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Snapshots one plugin's counters.
    #[must_use]
    pub fn snapshot(&self, plugin: &PluginId) -> PluginMetricsSnapshot {
        match self.plugins.read().expect("metrics lock").get(plugin) {
            Some(entry) => PluginMetricsSnapshot {
                activations: entry.activations.load(Ordering::Relaxed),
                failures: entry.failures.load(Ordering::Relaxed),
                events_published: entry.events_published.load(Ordering::Relaxed),
                events_dropped: entry.events_dropped.load(Ordering::Relaxed),
                commands_executed: entry.commands_executed.load(Ordering::Relaxed),
                last_load: Duration::from_millis(entry.last_load_millis.load(Ordering::Relaxed)),
            },
            None => PluginMetricsSnapshot::default(),
        }
    }

    /// The last recorded failure line for a plugin, if any.
    #[must_use]
    pub fn last_error(&self, plugin: &PluginId) -> Option<String> {
        self.plugins
            .read()
            .expect("metrics lock")
            .get(plugin)
            .and_then(|entry| entry.last_error.read().expect("metrics error lock").clone())
    }

    /// Plugins with recorded activity, sorted.
    #[must_use]
    pub fn tracked_plugins(&self) -> Vec<PluginId> {
        self.plugins
            .read()
            .expect("metrics lock")
            .keys()
            .cloned()
            .collect()
    }

    fn entry(&self, plugin: &PluginId) -> Arc<PerPlugin> {
        let mut plugins = self.plugins.write().expect("metrics lock");
        Arc::clone(plugins.entry(plugin.clone()).or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_track_per_plugin() {
        let a = PluginId::new("org.nexus.a.one").unwrap();
        let b = PluginId::new("org.nexus.a.two").unwrap();
        let metrics = PluginMetrics::default();
        metrics.record_activation(&a, Duration::from_millis(12));
        metrics.record_activation(&a, Duration::from_millis(13));
        metrics.record_failure(&b, "boom");
        let sa = metrics.snapshot(&a);
        assert_eq!(sa.activations, 2);
        assert_eq!(sa.last_load, Duration::from_millis(13));
        let sb = metrics.snapshot(&b);
        assert_eq!(sb.failures, 1);
        assert_eq!(metrics.last_error(&b).as_deref(), Some("boom"));
        assert_eq!(metrics.tracked_plugins(), vec![a, b]);
    }
}
