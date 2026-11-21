use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nexus_core::event::EventSequence;
use nexus_core::ids::{CubeId, RequestId, SessionId, SnapshotId};
use nexus_core::version::Version;
use nexus_events::store::EventStore;
use nexus_protocol::message::{
    EventRange, OperationReconciliationResult, OperationReconciliationStatus, PendingOperation,
    RecoveryPlan, RecoverySessionStatus, RecoveryStage, SyncStrategy,
};
use nexus_snapshot::metadata::SnapshotMetadata;
use nexus_snapshot::store::SnapshotStore;
use tokio::sync::RwLock;
use tracing::info;

use crate::workspace::WorkspaceManager;
use nexus_core::cube::CubeManager;
use nexus_core::error::Error;

/// Manages server-side recovery operations for Cubes and sessions.
pub struct RecoveryManager {
    cube_manager: Arc<dyn CubeManager>,
    event_store: Arc<dyn EventStore>,
    snapshot_store: Arc<dyn SnapshotStore>,
    #[allow(dead_code)]
    workspace_manager: Arc<WorkspaceManager>,
    recovery_state: Arc<RwLock<RecoveryState>>,
    config: RecoveryConfig,
}

/// Configuration for recovery behavior.
#[derive(Debug, Clone)]
pub struct RecoveryConfig {
    /// Maximum time to wait for session recovery.
    pub session_recovery_timeout: Duration,
    /// Maximum time for snapshot transfer.
    pub snapshot_transfer_timeout: Duration,
    /// Maximum time for event replay.
    pub event_replay_timeout: Duration,
    /// Maximum time for operation reconciliation.
    pub reconciliation_timeout: Duration,
    /// Maximum event gap to replay before using snapshot.
    pub max_event_gap_for_replay: u64,
}

impl Default for RecoveryConfig {
    fn default() -> Self {
        Self {
            session_recovery_timeout: Duration::from_secs(30),
            snapshot_transfer_timeout: Duration::from_secs(60),
            event_replay_timeout: Duration::from_secs(30),
            reconciliation_timeout: Duration::from_secs(10),
            max_event_gap_for_replay: 10000,
        }
    }
}

/// Tracks ongoing recovery operations.
#[derive(Debug, Default)]
struct RecoveryState {
    /// Active recovery operations by cube_id.
    active_recoveries: HashMap<CubeId, RecoveryOperation>,
}

/// An ongoing recovery operation.
#[derive(Debug)]
#[allow(dead_code)]
struct RecoveryOperation {
    cube_id: CubeId,
    session_id: Option<SessionId>,
    client_sequence: EventSequence,
    workspace_version: Version,
    started_at: Instant,
    current_stage: RecoveryStage,
    plan: Option<RecoveryPlan>,
}

impl RecoveryManager {
    /// Creates a new recovery manager.
    pub fn new(
        cube_manager: Arc<dyn CubeManager>,
        event_store: Arc<dyn EventStore>,
        snapshot_store: Arc<dyn SnapshotStore>,
        workspace_manager: Arc<WorkspaceManager>,
        config: RecoveryConfig,
    ) -> Self {
        Self {
            cube_manager,
            event_store,
            snapshot_store,
            workspace_manager,
            recovery_state: Arc::new(RwLock::new(RecoveryState::default())),
            config,
        }
    }

    /// Computes a recovery plan for a client.
    pub async fn compute_recovery_plan(
        &self,
        request_id: RequestId,
        session_id: Option<SessionId>,
        cube_id: CubeId,
        client_sequence: EventSequence,
        _workspace_version: Version,
        _pending_operation_ids: Vec<RequestId>,
    ) -> nexus_core::Result<RecoveryPlan> {
        // Get the cube's current state
        let _cube = self
            .cube_manager
            .get_cube(cube_id)
            .await?
            .ok_or_else(|| nexus_core::error::Error::not_found("cube not found"))?;

        // Get the latest snapshot
        let latest_snapshot = self
            .snapshot_store
            .get_latest(cube_id)
            .await
            .map_err(|e| Error::internal(format!("snapshot store error: {}", e)))?;

        // Get the current event sequence
        let current_sequence = self.get_current_event_sequence(cube_id).await?;

        // Determine synchronization strategy
        let sync_strategy = self.determine_sync_strategy(
            client_sequence,
            current_sequence,
            latest_snapshot.as_ref(),
        );

        let (snapshot_id, event_range) = match sync_strategy {
            SyncStrategy::NoSyncRequired => (None, None),
            SyncStrategy::EventReplay => {
                let from_seq = client_sequence.next();
                let to_seq = current_sequence;
                (
                    None,
                    Some(EventRange {
                        from_sequence: from_seq,
                        to_sequence: to_seq,
                    }),
                )
            }
            SyncStrategy::SnapshotPlusReplay => {
                let snapshot_id = latest_snapshot
                    .as_ref()
                    .map(|s| s.snapshot_id)
                    .ok_or_else(|| nexus_core::error::Error::not_found("no snapshot available"))?;
                let from_seq = latest_snapshot
                    .as_ref()
                    .map(|s| s.event_sequence.next())
                    .unwrap_or(client_sequence.next());
                (
                    Some(snapshot_id),
                    Some(EventRange {
                        from_sequence: from_seq,
                        to_sequence: current_sequence,
                    }),
                )
            }
            SyncStrategy::FullWorkspaceSync => {
                let snapshot_id = latest_snapshot
                    .as_ref()
                    .map(|s| s.snapshot_id)
                    .ok_or_else(|| nexus_core::error::Error::not_found("no snapshot available"))?;
                (Some(snapshot_id), None)
            }
        };

        // Determine session status
        let session_status = match session_id {
            Some(sid) => self.get_session_status(sid).await?,
            None => RecoverySessionStatus::Active,
        };

        Ok(RecoveryPlan {
            request_id,
            session_status,
            sync_strategy,
            snapshot_id,
            event_range,
            requires_full_sync: matches!(sync_strategy, SyncStrategy::FullWorkspaceSync),
        })
    }

    /// Determines the synchronization strategy based on client and server state.
    fn determine_sync_strategy(
        &self,
        client_sequence: EventSequence,
        current_sequence: EventSequence,
        latest_snapshot: Option<&SnapshotMetadata>,
    ) -> SyncStrategy {
        // If client is up to date
        if client_sequence >= current_sequence {
            return SyncStrategy::NoSyncRequired;
        }

        let gap = current_sequence
            .value()
            .saturating_sub(client_sequence.value());

        // If gap is small, use event replay
        if gap <= self.config.max_event_gap_for_replay {
            return SyncStrategy::EventReplay;
        }

        // If we have a valid snapshot, use snapshot + replay
        if latest_snapshot.is_some() {
            return SyncStrategy::SnapshotPlusReplay;
        }

        // Otherwise, full workspace sync
        SyncStrategy::FullWorkspaceSync
    }

    /// Gets the current event sequence for a cube.
    async fn get_current_event_sequence(
        &self,
        cube_id: CubeId,
    ) -> nexus_core::Result<EventSequence> {
        // Get the latest event sequence from the event store
        Ok(self.event_store.latest_sequence(cube_id))
    }

    /// Gets session status for recovery.
    async fn get_session_status(
        &self,
        _session_id: SessionId,
    ) -> nexus_core::Result<RecoverySessionStatus> {
        // Check session manager for session state
        // For now, return Active
        Ok(RecoverySessionStatus::Active)
    }

    /// Handles a RecoveryRequest and returns a RecoveryPlan.
    pub async fn handle_recovery_request(
        &self,
        request_id: RequestId,
        session_id: Option<SessionId>,
        cube_id: CubeId,
        client_sequence: EventSequence,
        workspace_version: Version,
        pending_operation_ids: Vec<RequestId>,
    ) -> nexus_core::Result<RecoveryPlan> {
        info!(
            cube_id = %cube_id,
            client_sequence = %client_sequence,
            "computing recovery plan"
        );

        self.compute_recovery_plan(
            request_id,
            session_id,
            cube_id,
            client_sequence,
            workspace_version,
            pending_operation_ids,
        )
        .await
    }

    /// Handles ReplayEvents request.
    pub async fn handle_replay_events(
        &self,
        cube_id: CubeId,
        from_sequence: EventSequence,
        to_sequence: Option<EventSequence>,
    ) -> nexus_core::Result<(Vec<nexus_core::event::Event>, bool)> {
        let _to_seq = match to_sequence {
            Some(seq) => seq,
            None => self.event_store.latest_sequence(cube_id),
        };

        let events = self
            .event_store
            .replay(cube_id, from_sequence)
            .map_err(|e| Error::internal(format!("event store error: {}", e)))?;

        let has_more = to_sequence.is_some() && events.len() >= 1000;

        Ok((events, has_more))
    }

    /// Handles RequestSnapshot request.
    pub async fn handle_request_snapshot(
        &self,
        _cube_id: CubeId,
        snapshot_id: SnapshotId,
        _offset: u64,
        _chunk_size: u32,
    ) -> nexus_core::Result<(Vec<u8>, bool)> {
        let snapshot = self.snapshot_store.read(snapshot_id).await.map_err(|e| {
            nexus_core::error::Error::internal(format!("snapshot store error: {}", e))
        })?;

        // For now, return the full snapshot data
        // In production, this would be chunked
        let data = postcard::to_stdvec(&snapshot).map_err(|e| {
            nexus_core::error::Error::internal(format!("serialization error: {}", e))
        })?;
        let is_final = true;

        Ok((data, is_final))
    }

    /// Handles ReconcileOperations request.
    pub async fn handle_reconcile_operations(
        &self,
        cube_id: CubeId,
        operations: Vec<PendingOperation>,
    ) -> nexus_core::Result<Vec<OperationReconciliationResult>> {
        let mut results = Vec::new();

        for op in operations {
            // Check if operation was already applied
            let already_applied = self
                .check_operation_applied(cube_id, op.operation_id)
                .await?;

            let status = if already_applied {
                OperationReconciliationStatus::Accepted
            } else {
                // Try to apply the operation
                match self.apply_pending_operation(cube_id, &op).await {
                    Ok(transformed) => {
                        if transformed.is_some() {
                            OperationReconciliationStatus::Merged
                        } else {
                            OperationReconciliationStatus::Accepted
                        }
                    }
                    Err(_) => OperationReconciliationStatus::Rejected,
                }
            };

            results.push(OperationReconciliationResult {
                operation_id: op.operation_id,
                status,
                transformed_operations: None,
            });
        }

        Ok(results)
    }

    /// Checks if an operation was already applied.
    async fn check_operation_applied(
        &self,
        _cube_id: CubeId,
        _operation_id: RequestId,
    ) -> nexus_core::Result<bool> {
        // In production, this would check the event store for operation acceptance events
        // For now, return false (operation not yet applied)
        Ok(false)
    }

    /// Applies a pending operation during reconciliation.
    async fn apply_pending_operation(
        &self,
        _cube_id: CubeId,
        _op: &PendingOperation,
    ) -> nexus_core::Result<Option<Vec<nexus_protocol::sync::TextOperation>>> {
        // This would integrate with the sync system to apply the operation
        // For now, return Ok(None) meaning accepted without transformation
        Ok(None)
    }

    /// Starts tracking a recovery operation.
    pub async fn start_recovery(
        &self,
        cube_id: CubeId,
        session_id: Option<SessionId>,
        client_sequence: EventSequence,
        workspace_version: Version,
    ) {
        let mut state = self.recovery_state.write().await;
        state.active_recoveries.insert(
            cube_id,
            RecoveryOperation {
                cube_id,
                session_id,
                client_sequence,
                workspace_version,
                started_at: Instant::now(),
                current_stage: RecoveryStage::SessionRecovery,
                plan: None,
            },
        );
    }

    /// Updates the recovery stage for a cube.
    pub async fn update_recovery_stage(&self, cube_id: CubeId, stage: RecoveryStage) {
        let mut state = self.recovery_state.write().await;
        if let Some(op) = state.active_recoveries.get_mut(&cube_id) {
            op.current_stage = stage;
        }
    }

    /// Completes a recovery operation.
    pub async fn complete_recovery(&self, cube_id: CubeId) {
        let mut state = self.recovery_state.write().await;
        state.active_recoveries.remove(&cube_id);
    }

    /// Gets the current recovery stage for a cube.
    pub async fn get_recovery_stage(&self, cube_id: CubeId) -> Option<RecoveryStage> {
        let state = self.recovery_state.read().await;
        state
            .active_recoveries
            .get(&cube_id)
            .map(|op| op.current_stage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::InMemoryCubeManager;
    use crate::storage::InMemoryStorage;
    use crate::workspace::WorkspaceManager;

    use std::sync::Arc;

    #[tokio::test]
    async fn test_recovery_manager_creation() {
        let cube_manager = Arc::new(InMemoryCubeManager::new(10));
        let event_store = Arc::new(nexus_events::store::MemoryEventStore::new());

        // Create temp directory for snapshot store
        let temp_dir = tempfile::tempdir().unwrap();
        let snapshot_store =
            Arc::new(nexus_snapshot::store::FileSnapshotStore::new(&temp_dir).unwrap());

        let storage = Arc::new(InMemoryStorage::new());
        let workspace_manager = Arc::new(WorkspaceManager::new(storage));

        let manager = RecoveryManager::new(
            cube_manager,
            event_store,
            snapshot_store,
            workspace_manager,
            RecoveryConfig::default(),
        );

        assert_eq!(manager.config.max_event_gap_for_replay, 10000);
    }
}
