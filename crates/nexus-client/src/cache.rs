//! Bounded workspace cache.
//!
//! Keeps recently opened file contents in memory with a simple LRU eviction
//! policy and a hard capacity, so browsing a large tree cannot exhaust memory.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use nexus_core::ids::FileId;

/// A thread-safe-in-practice, bounded in-memory file content cache.
#[derive(Debug, Clone)]
pub struct WorkspaceCache {
    capacity: usize,
    entries: HashMap<FileId, Entry>,
    /// Recency counter; larger = more recently used.
    clock: u64,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // `path`/`big` are stored for later rename & large-file flows
struct Entry {
    path: PathBuf,
    content: String,
    big: bool,
    touched: u64,
}

impl WorkspaceCache {
    /// Creates a cache holding up to `capacity` files.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: HashMap::new(),
            clock: 0,
        }
    }

    /// The total bytes cached.
    #[must_use]
    pub fn cached_bytes(&self) -> usize {
        self.entries.values().map(|e| e.content.len()).sum()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Retrieves a cached file, marking it as recently used.
    #[must_use]
    pub fn get(&mut self, file_id: FileId) -> Option<&str> {
        let entry = self.entries.get_mut(&file_id)?;
        self.clock += 1;
        entry.touched = self.clock;
        Some(&entry.content)
    }

    /// Whether a file id is cached (read-only).
    #[must_use]
    pub fn contains(&self, file_id: FileId) -> bool {
        self.entries.contains_key(&file_id)
    }

    /// Stores a file, evicting least-recently-used entries when over capacity.
    pub fn put(&mut self, file_id: FileId, path: PathBuf, content: String) {
        self.clock += 1;
        let entry = Entry {
            path,
            content,
            big: false,
            touched: self.clock,
        };
        self.entries.insert(file_id, entry);
        self.evict();
    }

    /// Removes a file from the cache.
    pub fn remove(&mut self, file_id: FileId) {
        self.entries.remove(&file_id);
    }

    /// Retains only the given file ids (used when the tree is re-listed).
    pub fn retain_only(&mut self, valid: &HashSet<FileId>) {
        self.entries.retain(|id, _| valid.contains(id));
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    fn evict(&mut self) {
        while self.entries.len() > self.capacity {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.touched)
                .map(|(id, _)| *id);
            if let Some(id) = oldest {
                self.entries.remove(&id);
            } else {
                break;
            }
        }
    }
}

impl Default for WorkspaceCache {
    fn default() -> Self {
        Self::with_capacity(64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_get_roundtrip() {
        let mut cache = WorkspaceCache::with_capacity(4);
        let id = FileId::from_u64(1);
        cache.put(id, PathBuf::from("a.rs"), "fn main() {}".into());
        assert!(cache.contains(id));
        assert_eq!(cache.get(id), Some("fn main() {}"));
        assert_eq!(cache.cached_bytes(), "fn main() {}".len());
    }

    #[test]
    fn cache_evicts_least_recently_used() {
        let mut cache = WorkspaceCache::with_capacity(2);
        let a = FileId::from_u64(1);
        let b = FileId::from_u64(2);
        let c = FileId::from_u64(3);
        cache.put(a, PathBuf::from("a"), "a".into());
        cache.put(b, PathBuf::from("b"), "b".into());
        // Touch `a` so it survives eviction.
        let _ = cache.get(a);
        cache.put(c, PathBuf::from("c"), "c".into());
        assert!(!cache.contains(b), "b evicted (LRU)");
        assert!(cache.contains(a));
        assert!(cache.contains(c));
    }

    #[test]
    fn remove_and_clear() {
        let mut cache = WorkspaceCache::with_capacity(4);
        let id = FileId::from_u64(7);
        cache.put(id, PathBuf::from("x"), "x".into());
        cache.remove(id);
        assert!(!cache.contains(id));
        cache.put(id, PathBuf::from("x"), "x".into());
        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn unknown_ids_return_none() {
        let mut cache = WorkspaceCache::with_capacity(1);
        assert_eq!(cache.get(FileId::from_u64(9)), None);
        assert!(!cache.contains(FileId::from_u64(9)));
    }
}
