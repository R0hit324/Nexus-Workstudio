use std::collections::HashMap;
use std::sync::Arc;

use nexus_core::event::EventSequence;
use nexus_core::ids::SnapshotId;
use nexus_core::timestamp::Timestamp;
use nexus_core::version::Version;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

/// Lightweight snapshot metadata for indexing and lookup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMetadata {
    pub snapshot_id: SnapshotId,
    pub cube_id: nexus_core::ids::CubeId,
    pub event_sequence: EventSequence,
    pub workspace_version: Version,
    pub created_at: Timestamp,
    pub size_bytes: u64,
    pub checksum: String,
    pub schema_version: u32,
    pub snapshot_type: crate::model::SnapshotType,
    pub status: crate::model::SnapshotStatus,
    pub labels: Vec<crate::model::SnapshotLabel>,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
}

impl Default for SnapshotMetadata {
    fn default() -> Self {
        Self {
            snapshot_id: SnapshotId::default(),
            cube_id: nexus_core::ids::CubeId::default(),
            event_sequence: EventSequence::UNASSIGNED,
            workspace_version: Version::new(0),
            created_at: Timestamp::from_millis(0),
            size_bytes: 0,
            checksum: String::new(),
            schema_version: 0,
            snapshot_type: crate::model::SnapshotType::Full,
            status: crate::model::SnapshotStatus::Creating,
            labels: Vec::new(),
            compressed_size: 0,
            uncompressed_size: 0,
        }
    }
}

impl SnapshotMetadata {
    /// Checks if the snapshot is protected (status or labels indicate protection).
    pub fn is_protected(&self) -> bool {
        self.status == crate::model::SnapshotStatus::Protected
            || self
                .labels
                .iter()
                .any(|l| l.key == "protected" && l.value == "true")
    }
}

impl SnapshotMetadata {
    /// Creates metadata from a snapshot.
    pub fn from_snapshot(snapshot: &crate::model::WorkspaceSnapshot) -> Self {
        Self {
            snapshot_id: snapshot.snapshot_id,
            cube_id: snapshot.cube_id,
            event_sequence: snapshot.event_sequence,
            workspace_version: snapshot.workspace_version,
            created_at: snapshot.created_at,
            size_bytes: snapshot.compressed_size,
            checksum: snapshot.checksum.clone(),
            schema_version: snapshot.schema_version,
            snapshot_type: snapshot.snapshot_type,
            status: snapshot.status,
            labels: snapshot.labels.clone(),
            compressed_size: snapshot.compressed_size,
            uncompressed_size: snapshot.uncompressed_size,
        }
    }
}

/// In-memory index for efficient snapshot lookup.
#[derive(Debug, Default)]
pub struct SnapshotIndex {
    // Map: cube_id -> (event_sequence -> snapshot_id)
    by_cube_sequence: Arc<RwLock<HashMap<nexus_core::ids::CubeId, Vec<IndexEntry>>>>,
    // Map: snapshot_id -> metadata
    by_id: Arc<RwLock<HashMap<SnapshotId, SnapshotMetadata>>>,
    // Map: cube_id -> latest snapshot_id
    latest_by_cube: Arc<RwLock<HashMap<nexus_core::ids::CubeId, SnapshotId>>>,
}

#[derive(Debug, Clone)]
struct IndexEntry {
    event_sequence: EventSequence,
    snapshot_id: SnapshotId,
    #[allow(dead_code)]
    created_at: Timestamp,
    status: crate::model::SnapshotStatus,
}

impl SnapshotIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts or updates snapshot metadata in the index.
    pub fn insert(&self, metadata: SnapshotMetadata) {
        let cube_id = metadata.cube_id;
        let snapshot_id = metadata.snapshot_id;
        let event_sequence = metadata.event_sequence;
        let created_at = metadata.created_at;
        let status = metadata.status;

        // Update by_id
        self.by_id.write().insert(snapshot_id, metadata);

        // Update by_cube_sequence
        let mut by_cube = self.by_cube_sequence.write();
        let entries = by_cube.entry(cube_id).or_default();
        // Remove existing entry for same sequence if exists
        entries.retain(|e| e.event_sequence != event_sequence);
        entries.push(IndexEntry {
            event_sequence,
            snapshot_id,
            created_at,
            status,
        });
        // Keep sorted by event_sequence descending for efficient latest lookup
        entries.sort_by(|a, b| b.event_sequence.cmp(&a.event_sequence));

        // Update latest_by_cube if this is the newest active snapshot
        if status == crate::model::SnapshotStatus::Active {
            let mut latest = self.latest_by_cube.write();
            let is_newer = latest
                .get(&cube_id)
                .map(|existing_id| {
                    if let Some(existing) = self.by_id.read().get(existing_id) {
                        event_sequence > existing.event_sequence
                    } else {
                        true
                    }
                })
                .unwrap_or(true);

            if is_newer {
                latest.insert(cube_id, snapshot_id);
            }
        }
    }

    /// Removes a snapshot from the index.
    pub fn remove(&self, snapshot_id: SnapshotId) {
        if let Some(metadata) = self.by_id.write().remove(&snapshot_id) {
            let cube_id = metadata.cube_id;
            let _event_sequence = metadata.event_sequence;

            // Remove from by_cube_sequence, releasing the write guard before
            // recompute_latest takes the read side again.
            let removed = {
                let mut cube_map = self.by_cube_sequence.write();
                if let Some(entries) = cube_map.get_mut(&cube_id) {
                    entries.retain(|e| e.snapshot_id != snapshot_id);
                    true
                } else {
                    false
                }
            };
            if removed && self.recompute_latest_if_gone(cube_id, &snapshot_id) {
                self.recompute_latest(cube_id);
            }
        }
    }

    /// Returns `true` if `snapshot_id` is currently recorded as latest.
    fn recompute_latest_if_gone(
        &self,
        cube_id: nexus_core::ids::CubeId,
        snapshot_id: &SnapshotId,
    ) -> bool {
        self.latest_by_cube
            .read()
            .get(&cube_id)
            .is_some_and(|latest| latest == snapshot_id)
    }

    fn recompute_latest(&self, cube_id: nexus_core::ids::CubeId) {
        let latest_id = self
            .by_cube_sequence
            .read()
            .get(&cube_id)
            .and_then(|entries| {
                entries
                    .iter()
                    .filter(|e| e.status == crate::model::SnapshotStatus::Active)
                    .max_by_key(|e| e.event_sequence)
                    .map(|e| e.snapshot_id)
            });

        if let Some(id) = latest_id {
            self.latest_by_cube.write().insert(cube_id, id);
        } else {
            self.latest_by_cube.write().remove(&cube_id);
        }
    }

    /// Gets the latest active snapshot ID for a cube.
    pub fn get_latest(&self, cube_id: nexus_core::ids::CubeId) -> Option<SnapshotId> {
        self.latest_by_cube.read().get(&cube_id).copied()
    }

    /// Gets metadata by snapshot ID.
    pub fn get(&self, snapshot_id: SnapshotId) -> Option<SnapshotMetadata> {
        self.by_id.read().get(&snapshot_id).cloned()
    }

    /// Gets all snapshots for a cube, sorted by event sequence (newest first).
    pub fn list_for_cube(&self, cube_id: nexus_core::ids::CubeId) -> Vec<SnapshotMetadata> {
        let entries = self.by_cube_sequence.read().get(&cube_id).cloned();
        if let Some(entries) = entries {
            entries
                .into_iter()
                .filter(|e| e.status != crate::model::SnapshotStatus::Deleted)
                .filter_map(|e| self.by_id.read().get(&e.snapshot_id).cloned())
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Gets snapshots in a sequence range for a cube.
    pub fn range_for_cube(
        &self,
        cube_id: nexus_core::ids::CubeId,
        min_sequence: EventSequence,
        max_sequence: EventSequence,
    ) -> Vec<SnapshotMetadata> {
        self.list_for_cube(cube_id)
            .into_iter()
            .filter(|m| m.event_sequence >= min_sequence && m.event_sequence <= max_sequence)
            .collect()
    }

    /// Finds the best snapshot for recovery (latest active at or before sequence).
    pub fn find_best_for_recovery(
        &self,
        cube_id: nexus_core::ids::CubeId,
        target_sequence: EventSequence,
    ) -> Option<SnapshotMetadata> {
        let entries = self.by_cube_sequence.read().get(&cube_id).cloned()?;
        entries
            .into_iter()
            .filter(|e| {
                e.status == crate::model::SnapshotStatus::Active
                    && e.event_sequence <= target_sequence
            })
            .max_by_key(|e| e.event_sequence)
            .and_then(|e| self.by_id.read().get(&e.snapshot_id).cloned())
    }

    /// Gets the count of snapshots for a cube.
    pub fn count_for_cube(&self, cube_id: nexus_core::ids::CubeId) -> usize {
        self.by_cube_sequence
            .read()
            .get(&cube_id)
            .map(|entries| {
                entries
                    .iter()
                    .filter(|e| e.status != crate::model::SnapshotStatus::Deleted)
                    .count()
            })
            .unwrap_or(0)
    }

    /// Returns all snapshot metadata values.
    pub fn values(&self) -> Vec<SnapshotMetadata> {
        self.by_id.read().values().cloned().collect()
    }

    /// Marks a snapshot as deleted.
    pub fn mark_deleted(&self, snapshot_id: SnapshotId) {
        let cube_id = {
            let mut guard = self.by_id.write();
            if let Some(metadata) = guard.get_mut(&snapshot_id) {
                metadata.status = crate::model::SnapshotStatus::Deleted;
                metadata.cube_id
            } else {
                return;
            }
        };

        if let Some(entries) = self.by_cube_sequence.write().get_mut(&cube_id) {
            for e in entries.iter_mut() {
                if e.snapshot_id == snapshot_id {
                    e.status = crate::model::SnapshotStatus::Deleted;
                }
            }
            // Recompute latest if needed
            if self.latest_by_cube.read().get(&cube_id) == Some(&snapshot_id) {
                self.recompute_latest(cube_id);
            }
        }
    }

    /// Gets all active snapshots across all cubes.
    pub fn all_active(&self) -> Vec<SnapshotMetadata> {
        self.by_id
            .read()
            .values()
            .filter(|m| m.status == crate::model::SnapshotStatus::Active)
            .cloned()
            .collect()
    }
}
