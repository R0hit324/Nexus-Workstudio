//! Workspace service: file listing + bounded content cache + dirty tracking.
//!
//! Authority over the file tree is the server; this service keeps a local
//! mirror and hands opened files to the editor.

use std::collections::HashSet;
use std::path::PathBuf;

use nexus_core::ids::{FileId, WorkspaceId};

use super::app::state::WorkspaceState;
use super::cache::WorkspaceCache;

/// Local workspace mirror for the active cube.
#[derive(Debug, Clone)]
pub struct WorkspaceService {
    pub state: WorkspaceState,
    cache: WorkspaceCache,
    /// Files with unsaved local edits.
    dirty: HashSet<FileId>,
}

impl Default for WorkspaceService {
    fn default() -> Self {
        Self {
            state: WorkspaceState::default(),
            cache: WorkspaceCache::with_capacity(64),
            dirty: HashSet::new(),
        }
    }
}

impl WorkspaceService {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the local file listing, dropping stale cache entries.
    pub fn set_listing(&mut self, workspace: WorkspaceId, files: Vec<(FileId, PathBuf)>) {
        let valid: HashSet<FileId> = files.iter().map(|(id, _)| *id).collect();
        self.cache.retain_only(&valid);
        self.dirty.retain(|id| valid.contains(id));
        self.state.apply_listing(workspace, files);
    }

    /// Caches `content` for a file (big files may be skipped by the caller).
    pub fn cache_content(&mut self, file_id: FileId, path: PathBuf, content: String) {
        self.cache.put(file_id, path, content);
    }

    /// Reads cached content for a file.
    #[must_use]
    pub fn cached(&mut self, file_id: FileId) -> Option<&str> {
        self.cache.get(file_id)
    }

    /// Marks a file as having unsaved local edits.
    pub fn mark_dirty(&mut self, file_id: FileId) {
        self.dirty.insert(file_id);
    }

    /// Clears the dirty flag after a successful save.
    pub fn mark_saved(&mut self, file_id: FileId) {
        self.dirty.remove(&file_id);
    }

    #[must_use]
    pub fn is_dirty(&self, file_id: FileId) -> bool {
        self.dirty.contains(&file_id)
    }

    /// Fetches a file's path, if listed.
    #[must_use]
    pub fn path_of(&self, file_id: FileId) -> Option<&PathBuf> {
        self.state.path_of(file_id)
    }

    #[must_use]
    pub fn dirty_count(&self) -> usize {
        self.dirty.len()
    }

    /// Total bytes resident in the cache.
    #[must_use]
    pub fn cached_bytes(&self) -> usize {
        self.cache.cached_bytes()
    }
}
