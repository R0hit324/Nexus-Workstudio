use std::collections::VecDeque;
use std::time::Duration;

use nexus_core::event::EventSequence;
use nexus_core::ids::SnapshotId;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

/// Retention policy for snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionPolicy {
    /// Keep the latest N snapshots.
    pub keep_latest: usize,
    /// Keep snapshots younger than this duration.
    pub keep_younger_than: Option<Duration>,
    /// Maximum number of snapshots per cube.
    pub max_per_cube: usize,
    /// Keep at least one snapshot per N sequences.
    pub keep_per_sequence_interval: Option<EventSequence>,
    /// Protected snapshots are never deleted.
    pub respect_protected: bool,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            keep_latest: 5,
            keep_younger_than: Some(Duration::from_secs(7 * 24 * 60 * 60)), // 7 days
            max_per_cube: 20,
            keep_per_sequence_interval: Some(EventSequence::new(10000)),
            respect_protected: true,
        }
    }
}

impl RetentionPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_keep_latest(mut self, n: usize) -> Self {
        self.keep_latest = n;
        self
    }

    pub fn with_max_per_cube(mut self, n: usize) -> Self {
        self.max_per_cube = n;
        self
    }

    pub fn with_keep_younger_than(mut self, duration: Duration) -> Self {
        self.keep_younger_than = Some(duration);
        self
    }

    pub fn with_sequence_interval(mut self, interval: EventSequence) -> Self {
        self.keep_per_sequence_interval = Some(interval);
        self
    }

    pub fn without_protection(mut self) -> Self {
        self.respect_protected = false;
        self
    }
}

/// Result of applying retention policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionResult {
    pub deleted: Vec<SnapshotId>,
    pub kept: Vec<SnapshotId>,
    pub skipped_protected: Vec<SnapshotId>,
    pub skipped_sequence_anchor: Vec<SnapshotId>,
    pub total_snapshots: usize,
}

/// Retention manager for applying policies.
#[derive(Debug)]
pub struct RetentionManager {
    policy: RetentionPolicy,
    deleted_history: RwLock<VecDeque<RetentionEvent>>,
}

/// A record of a snapshot removed by retention policy.
#[derive(Debug, Clone)]
pub struct RetentionEvent {
    pub snapshot_id: SnapshotId,
    pub cube_id: nexus_core::ids::CubeId,
    pub reason: String,
    pub timestamp: nexus_core::timestamp::Timestamp,
}

impl RetentionManager {
    pub fn new(policy: RetentionPolicy) -> Self {
        Self {
            policy,
            deleted_history: RwLock::new(VecDeque::new()),
        }
    }

    pub fn with_policy(mut self, policy: RetentionPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Applies retention policy to a set of snapshots.
    pub fn apply(&self, snapshots: &[crate::metadata::SnapshotMetadata]) -> RetentionResult {
        let mut deleted = Vec::new();
        let mut kept = Vec::new();
        let mut skipped_protected = Vec::new();
        let mut skipped_sequence_anchor = Vec::new();

        // Group by cube
        let mut by_cube: std::collections::HashMap<_, Vec<_>> = std::collections::HashMap::new();
        for snap in snapshots {
            if snap.status == crate::model::SnapshotStatus::Deleted {
                continue;
            }
            by_cube.entry(snap.cube_id).or_default().push(snap);
        }

        for (cube_id, mut cube_snapshots) in by_cube {
            // Sort by event sequence descending (newest first)
            cube_snapshots.sort_by(|a, b| b.event_sequence.cmp(&a.event_sequence));

            let mut sequence_anchors = std::collections::HashSet::new();

            for (idx, snap) in cube_snapshots.iter().enumerate() {
                let mut should_keep = false;
                let mut skip_reason = None;

                // Always keep protected snapshots
                if self.policy.respect_protected && snap.is_protected() {
                    should_keep = true;
                    skip_reason = Some("protected".to_string());
                }

                // Keep latest N
                if idx < self.policy.keep_latest {
                    should_keep = true;
                    skip_reason = Some("latest".to_string());
                }

                // Keep younger than duration
                if let Some(max_age) = self.policy.keep_younger_than {
                    let now_ms = nexus_core::timestamp::current_time_ms();
                    let age =
                        Duration::from_millis(now_ms.saturating_sub(snap.created_at.as_millis()));
                    if age < max_age {
                        should_keep = true;
                        skip_reason = Some("recent".to_string());
                    }
                }

                // Keep sequence anchors
                if let Some(interval) = self.policy.keep_per_sequence_interval {
                    if snap.event_sequence.value() % interval.value() == 0 {
                        if sequence_anchors.insert(snap.event_sequence) {
                            should_keep = true;
                            skip_reason = Some("sequence_anchor".to_string());
                        }
                    }
                }

                // Max per cube
                if kept
                    .iter()
                    .filter(|k| {
                        snapshots
                            .iter()
                            .find(|s| s.snapshot_id == **k)
                            .map(|s| s.cube_id)
                            == Some(cube_id)
                    })
                    .count()
                    >= self.policy.max_per_cube
                    && !should_keep
                {
                    // Will be deleted
                }

                if should_keep {
                    kept.push(snap.snapshot_id);
                    if let Some(reason) = skip_reason {
                        if reason == "protected" {
                            skipped_protected.push(snap.snapshot_id);
                        } else if reason == "sequence_anchor" {
                            skipped_sequence_anchor.push(snap.snapshot_id);
                        }
                    }
                } else {
                    deleted.push(snap.snapshot_id);
                }
            }
        }

        RetentionResult {
            deleted: deleted.clone(),
            kept: kept.clone(),
            skipped_protected,
            skipped_sequence_anchor,
            total_snapshots: snapshots.len(),
        }
    }

    /// Gets the policy.
    pub fn policy(&self) -> &RetentionPolicy {
        &self.policy
    }

    /// Updates the policy.
    pub fn set_policy(&mut self, policy: RetentionPolicy) {
        self.policy = policy;
    }

    /// Records a deletion event.
    pub fn record_deletion(
        &self,
        snapshot_id: SnapshotId,
        cube_id: nexus_core::ids::CubeId,
        reason: String,
    ) {
        self.deleted_history
            .write()
            .push_back(crate::retention::RetentionEvent {
                snapshot_id,
                cube_id,
                reason,
                timestamp: nexus_core::timestamp::Timestamp::now(),
            });
    }

    /// Gets deletion history.
    pub fn history(&self) -> Vec<RetentionEvent> {
        self.deleted_history.read().iter().cloned().collect()
    }
}

impl Default for RetentionManager {
    fn default() -> Self {
        Self::new(RetentionPolicy::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retention_policy_defaults() {
        let policy = RetentionPolicy::default();
        assert_eq!(policy.keep_latest, 5);
        assert_eq!(policy.max_per_cube, 20);
        assert!(policy.respect_protected);
    }
}
