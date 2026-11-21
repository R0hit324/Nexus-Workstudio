//! Workspace file watcher for detecting external changes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use nexus_core::{CubeId, FileId, Timestamp};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::RwLock;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::events::{FileChangeKind, WorkspaceChangeEvent};

/// Configuration for the workspace watcher.
#[derive(Debug, Clone)]
pub struct WatcherConfig {
    /// Debounce duration for events
    pub debounce: Duration,
    /// Maximum events per batch
    pub max_batch_size: usize,
    /// Ignore patterns (glob)
    pub ignore_patterns: Vec<String>,
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(100),
            max_batch_size: 100,
            ignore_patterns: vec![
                "target/**".into(),
                "node_modules/**".into(),
                ".git/**".into(),
                "*.tmp".into(),
                "*.swp".into(),
            ],
        }
    }
}

/// File change event from watcher.
#[derive(Debug, Clone)]
pub struct FileChange {
    pub cube_id: CubeId,
    pub path: PathBuf,
    pub kind: FileChangeKind,
    pub timestamp: Timestamp,
}

/// Watches workspace directories for file changes.
pub struct WorkspaceWatcher {
    #[allow(dead_code)]
    config: WatcherConfig,
    watcher: Option<RecommendedWatcher>,
    watch_paths: Arc<RwLock<HashMap<CubeId, PathBuf>>>,
    #[allow(dead_code)]
    event_tx: mpsc::UnboundedSender<WorkspaceChangeEvent>,
    #[allow(dead_code)]
    shutdown_tx: tokio::sync::broadcast::Sender<()>,
}

impl WorkspaceWatcher {
    /// Creates a new workspace watcher.
    pub fn new(
        config: WatcherConfig,
        event_tx: mpsc::UnboundedSender<WorkspaceChangeEvent>,
    ) -> Self {
        let (shutdown_tx, _shutdown_rx) = tokio::sync::broadcast::channel(16);
        Self {
            config,
            watcher: None,
            watch_paths: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
            shutdown_tx,
        }
    }

    /// Starts watching a workspace for a cube.
    pub fn watch(&mut self, cube_id: CubeId, workspace_path: PathBuf) -> Result<(), anyhow::Error> {
        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                match res {
                    Ok(event) => {
                        // In real implementation, would send to channel
                        debug!(event = ?event, "File change detected");
                    }
                    Err(e) => warn!(error = %e, "Watcher error"),
                }
            },
            notify::Config::default(),
        )?;

        watcher.watch(&workspace_path, RecursiveMode::Recursive)?;

        self.watch_paths
            .write()
            .insert(cube_id, workspace_path.clone());
        self.watcher = Some(watcher);

        info!(%cube_id, path = %workspace_path.display(), "Started watching workspace");
        Ok(())
    }

    /// Stops watching a workspace.
    pub fn unwatch(&mut self, cube_id: CubeId) {
        if let Some(mut watcher) = self.watcher.take() {
            if let Some(path) = self.watch_paths.write().remove(&cube_id) {
                let _ = watcher.unwatch(&path);
            }
            self.watcher = Some(watcher);
        }
    }

    /// Processes a raw notify event into workspace change events.
    #[allow(dead_code)]
    fn process_event(&self, cube_id: CubeId, event: Event) -> Vec<WorkspaceChangeEvent> {
        let mut changes = Vec::new();

        for path in event.paths {
            if self.should_ignore(&path) {
                continue;
            }

            let kind = match event.kind {
                EventKind::Create(_) => FileChangeKind::Created,
                EventKind::Modify(_) => FileChangeKind::Modified,
                EventKind::Remove(_) => FileChangeKind::Deleted,
                EventKind::Any => FileChangeKind::Modified,
                _ => continue,
            };

            // In real implementation, would resolve file_id from workspace
            let file_id = FileId::new();

            changes.push(WorkspaceChangeEvent {
                cube_id,
                file_id,
                path: path.clone(),
                kind,
                timestamp: Timestamp::now(),
            });
        }

        changes
    }

    /// Checks if a path should be ignored.
    #[allow(dead_code)]
    fn should_ignore(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy();
        for pattern in &self.config.ignore_patterns {
            if glob::Pattern::new(pattern)
                .map(|p| p.matches(&path_str))
                .unwrap_or(false)
            {
                return true;
            }
        }
        false
    }

    /// Gets all watched paths.
    pub fn watched_paths(&self) -> HashMap<CubeId, PathBuf> {
        self.watch_paths.read().clone()
    }
}

/// Events module for workspace changes.
pub mod events {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ignore_patterns() {
        let config = WatcherConfig::default();
        let watcher = WorkspaceWatcher {
            config,
            watcher: None,
            watch_paths: Default::default(),
            event_tx: tokio::sync::mpsc::unbounded_channel().0,
            shutdown_tx: tokio::sync::broadcast::channel(1).0,
        };

        assert!(watcher.should_ignore(Path::new("target/debug/main")));
        assert!(watcher.should_ignore(Path::new("node_modules/package/index.js")));
        assert!(watcher.should_ignore(Path::new(".git/config")));
        assert!(!watcher.should_ignore(Path::new("src/main.rs")));
    }
}
