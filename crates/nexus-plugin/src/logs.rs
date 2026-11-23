//! Plugin logging (spec §73): bounded, per-plugin, and never the host's
//! log. Entries are kept in a per-plugin ring so a chatty plugin cannot
//! fill the host's memory or interleave another plugin's diagnostics.

use std::collections::{BTreeMap, VecDeque};
use std::sync::RwLock;

use crate::id::PluginId;

/// Entries retained per plugin before the ring wraps.
pub const LOG_ENTRIES_PER_PLUGIN: usize = 128;

/// Severity of a plugin log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PluginLogLevel {
    /// Verbose detail.
    Trace,
    /// Routine internal events.
    Debug,
    /// Notable progress.
    Info,
    /// Something degraded — plugin keeps running.
    Warn,
    /// Actionable plugin error surfaced to the host.
    Error,
}

impl PluginLogLevel {
    /// Short tag (`INF`).
    #[must_use]
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Trace => "TRC",
            Self::Debug => "DBG",
            Self::Info => "INF",
            Self::Warn => "WRN",
            Self::Error => "ERR",
        }
    }
}

/// One plugin log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginLogEntry {
    /// Emitting plugin.
    pub plugin_id: PluginId,
    /// Level.
    pub level: PluginLogLevel,
    /// Message (single-line; control characters stripped at write time).
    pub message: String,
    /// ms timestamp from the standard epoch.
    pub timestamp_ms: u64,
}

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Bounded per-plugin log ring.
#[derive(Default)]
pub struct PluginLogBuffer {
    entries: RwLock<BTreeMap<PluginId, VecDeque<PluginLogEntry>>>,
    dropped: RwLock<BTreeMap<PluginId, u64>>,
}

fn sanitize(message: &str) -> String {
    message
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_string()
}

impl PluginLogBuffer {
    /// Builds an empty buffer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends one entry, evicting the oldest when the ring fills.
    pub fn log(&self, plugin_id: &PluginId, level: PluginLogLevel, message: &str) {
        let message = sanitize(message);
        if message.is_empty() {
            return;
        }
        let entry = PluginLogEntry {
            plugin_id: plugin_id.clone(),
            level,
            message,
            timestamp_ms: unix_millis(),
        };
        let mut rings = self.entries.write().expect("plugin log lock");
        let ring = rings.entry(plugin_id.clone()).or_default();
        while ring.len() >= LOG_ENTRIES_PER_PLUGIN {
            ring.pop_front();
            *self
                .dropped
                .write()
                .expect("plugin log lock")
                .entry(plugin_id.clone())
                .or_default() += 1;
        }
        ring.push_back(entry);
    }

    /// Newest last.
    #[must_use]
    pub fn entries_of(&self, plugin_id: &PluginId) -> Vec<PluginLogEntry> {
        self.entries
            .read()
            .expect("plugin log lock")
            .get(plugin_id)
            .map(|ring| ring.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// How many entries were evicted for this plugin.
    #[must_use]
    pub fn dropped(&self, plugin_id: &PluginId) -> u64 {
        self.dropped
            .read()
            .expect("plugin log lock")
            .get(plugin_id)
            .copied()
            .unwrap_or(0)
    }

    /// Clears one plugin's ring (invoked on disable/uninstall).
    pub fn clear(&self, plugin_id: &PluginId) {
        self.entries
            .write()
            .expect("plugin log lock")
            .remove(plugin_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(name: &str) -> PluginId {
        PluginId::new(name).unwrap()
    }

    #[test]
    fn ring_keeps_newest_and_counts_drops() {
        let buffer = PluginLogBuffer::new();
        let id = pid("org.nexus.log.talker");
        for index in 0..(LOG_ENTRIES_PER_PLUGIN + 5) {
            buffer.log(&id, PluginLogLevel::Info, &format!("line-{index}"));
        }
        let entries = buffer.entries_of(&id);
        assert_eq!(entries.len(), LOG_ENTRIES_PER_PLUGIN);
        assert_eq!(entries[LOG_ENTRIES_PER_PLUGIN - 1].message, "line-132");
        assert_eq!(entries[0].message, "line-5", "oldest lines evicted first");
        assert_eq!(
            buffer.dropped(&id),
            (LOG_ENTRIES_PER_PLUGIN + 5 - LOG_ENTRIES_PER_PLUGIN) as u64
        );
    }

    #[test]
    fn plugins_never_see_each_others_logs() {
        let buffer = PluginLogBuffer::new();
        let a = pid("org.nexus.log.one");
        let b = pid("org.nexus.log.two");
        buffer.log(&a, PluginLogLevel::Error, "only-a");
        buffer.log(&b, PluginLogLevel::Error, "only-b");
        let a_entries = buffer.entries_of(&a);
        assert_eq!(a_entries.len(), 1);
        assert_eq!(a_entries[0].message, "only-a");
        buffer.clear(&a);
        buffer.clear(&b);
        assert!(buffer.entries_of(&b).is_empty());
    }

    #[test]
    fn messages_filtered_and_flattened() {
        let buffer = PluginLogBuffer::new();
        let id = pid("org.nexus.log.fresh");
        buffer.log(&id, PluginLogLevel::Warn, "\nspilled\nline\n");
        let entries = buffer.entries_of(&id);
        assert_eq!(entries[0].message, "spilled line");
        buffer.log(&id, PluginLogLevel::Debug, "   ");
        assert_eq!(buffer.entries_of(&id).len(), 1, "blank skipped");
    }

    #[test]
    fn level_tags_are_stable() {
        assert_eq!(PluginLogLevel::Error.tag(), "ERR");
        assert_eq!(PluginLogLevel::Trace.tag(), "TRC");
        assert!(PluginLogLevel::Error > PluginLogLevel::Info);
    }
}
