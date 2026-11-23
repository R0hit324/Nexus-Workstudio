use std::sync::Arc;
use std::time::{Duration, Instant};

use nexus_core::event::EventSequence;
use nexus_core::ids::{CubeId, RequestId, SessionId};
use nexus_core::timestamp::Timestamp;
use nexus_core::version::Version;
use nexus_protocol::message::{
    OperationReconciliationResult, PendingOperation, RecoveryPlan, RecoveryStage, SyncStrategy,
};
use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::{debug, info, warn};

use crate::app::state::{ClientState, ConnectionState};
use crate::client::Client;

/// Configuration for client recovery behavior.
#[derive(Debug, Clone)]
pub struct RecoveryConfig {
    /// Minimum delay between reconnection attempts.
    pub min_reconnect_delay: Duration,
    /// Maximum delay between reconnection attempts.
    pub max_reconnect_delay: Duration,
    /// Maximum number of reconnection attempts before giving up.
    pub max_reconnect_attempts: u32,
    /// Base delay for exponential backoff.
    pub base_reconnect_delay: Duration,
    /// Jitter factor (0.0 to 1.0) to add randomness to backoff.
    pub jitter_factor: f64,
    /// Heartbeat timeout for failure detection.
    pub heartbeat_timeout: Duration,
    /// Timeout for session recovery.
    pub session_recovery_timeout: Duration,
    /// Timeout for snapshot transfer.
    pub snapshot_transfer_timeout: Duration,
    /// Timeout for event replay.
    pub event_replay_timeout: Duration,
    /// Timeout for operation reconciliation.
    pub reconciliation_timeout: Duration,
}

impl Default for RecoveryConfig {
    fn default() -> Self {
        Self {
            min_reconnect_delay: Duration::from_millis(100),
            max_reconnect_delay: Duration::from_secs(30),
            max_reconnect_attempts: 10,
            base_reconnect_delay: Duration::from_millis(100),
            jitter_factor: 0.1,
            heartbeat_timeout: Duration::from_secs(30),
            session_recovery_timeout: Duration::from_secs(30),
            snapshot_transfer_timeout: Duration::from_secs(60),
            event_replay_timeout: Duration::from_secs(30),
            reconciliation_timeout: Duration::from_secs(10),
        }
    }
}

/// Client recovery state machine states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryState {
    /// Normal operation - fully synchronized.
    Ready,
    /// Connection lost, preserving local state.
    ConnectionLost,
    /// Actively attempting to reconnect.
    Reconnecting,
    /// Reconnected, recovering session.
    SessionRecovery,
    /// Session recovered, recovering Cube state.
    CubeRecovery,
    /// Cube recovered, synchronizing events/snapshots.
    SyncRecovery,
    /// Synchronized, reconciling pending operations.
    OperationReconciliation,
    /// Recovery failed, manual intervention required.
    RecoveryFailed,
}

/// A pending operation waiting for reconciliation.
#[derive(Debug, Clone)]
pub struct PendingOperationEntry {
    pub operation_id: RequestId,
    pub file_id: nexus_core::ids::FileId,
    pub base_version: Version,
    pub payload: Vec<u8>,
    pub created_at: Timestamp,
    pub retry_count: u32,
}

/// State preserved during disconnection for recovery.
#[derive(Debug, Clone)]
pub struct PreservedState {
    pub cube_id: Option<CubeId>,
    pub workspace_version: Version,
    pub last_confirmed_sequence: EventSequence,
    pub pending_operations: Vec<PendingOperationEntry>,
    pub open_files: Vec<(nexus_core::ids::FileId, std::path::PathBuf)>,
}

/// Recovery progress information.
#[derive(Debug, Clone)]
pub struct RecoveryProgress {
    pub stage: RecoveryStage,
    pub progress: u32,
    pub total: u32,
    pub message: String,
}

/// Client-side recovery manager.
pub struct RecoveryManager {
    config: RecoveryConfig,
    state: Arc<RwLock<RecoveryState>>,
    preserved_state: Arc<RwLock<Option<PreservedState>>>,
    client_state: Arc<RwLock<ClientState>>,
    reconnect_attempt: Arc<RwLock<u32>>,
    last_heartbeat_received: Arc<RwLock<Option<Instant>>>,
    recovery_progress: Arc<RwLock<Option<RecoveryProgress>>>,
    shutdown: Arc<RwLock<bool>>,
}

impl RecoveryManager {
    /// Creates a new recovery manager.
    pub fn new(config: RecoveryConfig, client_state: Arc<RwLock<ClientState>>) -> Self {
        Self {
            config,
            state: Arc::new(RwLock::new(RecoveryState::Ready)),
            preserved_state: Arc::new(RwLock::new(None)),
            client_state,
            reconnect_attempt: Arc::new(RwLock::new(0)),
            last_heartbeat_received: Arc::new(RwLock::new(None)),
            recovery_progress: Arc::new(RwLock::new(None)),
            shutdown: Arc::new(RwLock::new(false)),
        }
    }

    /// Gets the current recovery state.
    pub async fn state(&self) -> RecoveryState {
        *self.state.read().await
    }

    /// Gets the current recovery progress.
    pub async fn progress(&self) -> Option<RecoveryProgress> {
        self.recovery_progress.read().await.clone()
    }

    /// Records a heartbeat received from the server.
    pub async fn record_heartbeat(&self) {
        *self.last_heartbeat_received.write().await = Some(Instant::now());
    }

    /// Checks if the connection is healthy based on heartbeat timeout.
    pub async fn is_connection_healthy(&self) -> bool {
        if let Some(last) = *self.last_heartbeat_received.read().await {
            last.elapsed() < self.config.heartbeat_timeout
        } else {
            true // No heartbeat yet, assume healthy
        }
    }

    /// Called when the connection is lost.
    pub async fn on_connection_lost(&self, reason: Option<String>) {
        info!(?reason, "Connection lost, entering recovery mode");

        // Preserve local state before clearing connection
        self.preserve_local_state().await;

        // Update connection state
        {
            let mut client_state = self.client_state.write().await;
            client_state.connection = ConnectionState::Disconnected;
        }

        // Transition to connection lost state
        *self.state.write().await = RecoveryState::ConnectionLost;

        // Start reconnection process
        self.start_reconnection().await;
    }

    /// Preserves local state before disconnection.
    async fn preserve_local_state(&self) {
        let client_state = self.client_state.read().await;

        let preserved = PreservedState {
            cube_id: client_state.active_cube.as_ref().map(|c| c.id),
            workspace_version: client_state
                .workspace
                .workspace
                .map(|_| Version::new(1)) // Would get actual version from workspace
                .unwrap_or_else(|| Version::new(0)),
            last_confirmed_sequence: EventSequence::new(0), // Would track actual sequence
            pending_operations: Vec::new(),                 // Would collect from operation queue
            open_files: client_state.workspace.files.clone(),
        };

        *self.preserved_state.write().await = Some(preserved);

        debug!("Local state preserved for recovery");
    }

    /// Starts the reconnection process with exponential backoff.
    async fn start_reconnection(&self) {
        let mut attempt = self.reconnect_attempt.write().await;
        *attempt = 0;
        drop(attempt);

        *self.state.write().await = RecoveryState::Reconnecting;

        // Spawn reconnection task
        let manager = self.clone();
        tokio::spawn(async move {
            manager.reconnect_loop().await;
        });
    }

    /// Main reconnection loop with exponential backoff.
    async fn reconnect_loop(&self) {
        loop {
            // Check for shutdown
            if *self.shutdown.read().await {
                return;
            }

            let attempt = *self.reconnect_attempt.read().await;
            if attempt >= self.config.max_reconnect_attempts {
                warn!("Max reconnection attempts reached, giving up");
                *self.state.write().await = RecoveryState::RecoveryFailed;
                return;
            }

            // Calculate delay with exponential backoff and jitter
            let delay = self.calculate_backoff(attempt);

            info!(
                attempt = attempt + 1,
                delay_ms = delay.as_millis(),
                "Attempting reconnection"
            );

            *self.state.write().await = RecoveryState::Reconnecting;
            self.update_progress(RecoveryStage::SessionRecovery, 0, 1, "Reconnecting...")
                .await;

            sleep(delay).await;

            // Check for shutdown after sleep
            if *self.shutdown.read().await {
                return;
            }

            // Attempt to connect (would use actual client in real implementation)
            match self.attempt_reconnect().await {
                Ok(_) => {
                    info!("Reconnection successful");
                    break;
                }
                Err(e) => {
                    warn!(error = %e, "Reconnection failed, will retry");
                    *self.reconnect_attempt.write().await += 1;
                }
            }
        }
    }

    /// Calculates the backoff delay for a given attempt.
    fn calculate_backoff(&self, attempt: u32) -> Duration {
        let base = self.config.base_reconnect_delay.as_millis() as f64;
        let exp_delay = base * 2.0_f64.powi(attempt as i32);
        let delay_ms = (exp_delay as u64).min(self.config.max_reconnect_delay.as_millis() as u64);

        // Add jitter
        let jitter = (delay_ms as f64 * self.config.jitter_factor) as u64;
        let jitter_range = jitter.saturating_mul(2);
        let actual_jitter = if jitter_range > 0 {
            rand::random::<u64>() % jitter_range
        } else {
            0
        };
        let final_delay = delay_ms
            .saturating_sub(jitter)
            .max(self.config.min_reconnect_delay.as_millis() as u64)
            .saturating_add(actual_jitter);

        Duration::from_millis(final_delay.min(self.config.max_reconnect_delay.as_millis() as u64))
    }

    /// Attempts to reconnect to the server.
    async fn attempt_reconnect(&self) -> Result<(), String> {
        // In real implementation, this would create a new Client and connect
        // For now, simulate success
        // let client = Client::connect(&config).await?;
        // self.handle_successful_reconnect(client).await;
        Ok(())
    }

    /// Handles successful reconnection - starts recovery process.
    #[allow(dead_code)]
    async fn handle_successful_reconnect(
        &self,
        _client: Client,
        _session_id: Option<SessionId>,
        _cube_id: CubeId,
    ) {
        *self.reconnect_attempt.write().await = 0;
        *self.state.write().await = RecoveryState::SessionRecovery;
        self.update_progress(
            RecoveryStage::SessionRecovery,
            10,
            100,
            "Recovering session...",
        )
        .await;

        // Send RecoveryRequest to server
        let preserved = self.preserved_state.read().await;
        let _client_sequence = preserved
            .as_ref()
            .map(|p| p.last_confirmed_sequence)
            .unwrap_or_else(|| EventSequence::new(0));
        let _workspace_version = preserved
            .as_ref()
            .map(|p| p.workspace_version)
            .unwrap_or_else(|| Version::new(0));
        let _pending_ops: Vec<RequestId> = preserved
            .as_ref()
            .map(|p| {
                p.pending_operations
                    .iter()
                    .map(|op| op.operation_id)
                    .collect()
            })
            .unwrap_or_default();
        drop(preserved);

        // In real implementation, send RecoveryRequest and handle RecoveryPlan
        // let plan = client.send_recovery_request(request_id, session_id, cube_id, client_sequence, workspace_version, pending_ops).await?;

        // Process recovery plan
        // self.process_recovery_plan(plan, client, cube_id).await;

        *self.state.write().await = RecoveryState::Ready;
        self.update_progress(RecoveryStage::Complete, 100, 100, "Recovery complete")
            .await;
    }

    /// Processes the recovery plan from the server.
    #[allow(dead_code)]
    async fn process_recovery_plan(&self, plan: RecoveryPlan, _client: Client, _cube_id: CubeId) {
        match plan.sync_strategy {
            SyncStrategy::NoSyncRequired => {
                info!("No sync required, already up to date");
                self.update_progress(RecoveryStage::Complete, 100, 100, "Already synchronized")
                    .await;
            }
            SyncStrategy::EventReplay => {
                *self.state.write().await = RecoveryState::SyncRecovery;
                self.update_progress(RecoveryStage::EventReplay, 20, 100, "Replaying events...")
                    .await;

                if let Some(_range) = plan.event_range {
                    // Request event replay
                    // let events = client.replay_events(cube_id, range.from_sequence, range.to_sequence).await?;
                    // Apply events
                }
            }
            SyncStrategy::SnapshotPlusReplay => {
                *self.state.write().await = RecoveryState::CubeRecovery;
                self.update_progress(
                    RecoveryStage::SnapshotTransfer,
                    30,
                    100,
                    "Downloading snapshot...",
                )
                .await;

                if let Some(_snapshot_id) = plan.snapshot_id {
                    // Download snapshot
                    // let snapshot = client.request_snapshot(cube_id, snapshot_id).await?;
                    // Apply snapshot
                }

                // Then replay events
                if let Some(_range) = plan.event_range {
                    self.update_progress(
                        RecoveryStage::EventReplay,
                        70,
                        100,
                        "Replaying events...",
                    )
                    .await;
                    // let events = client.replay_events(cube_id, range.from_sequence, range.to_sequence).await?;
                }
            }
            SyncStrategy::FullWorkspaceSync => {
                *self.state.write().await = RecoveryState::CubeRecovery;
                self.update_progress(
                    RecoveryStage::SnapshotTransfer,
                    50,
                    100,
                    "Full workspace sync...",
                )
                .await;

                if let Some(_snapshot_id) = plan.snapshot_id {
                    // Download full snapshot
                    // let snapshot = client.request_snapshot(cube_id, snapshot_id).await?;
                }
            }
        }

        // Reconcile pending operations
        *self.state.write().await = RecoveryState::OperationReconciliation;
        self.update_progress(
            RecoveryStage::OperationReconciliation,
            90,
            100,
            "Reconciling operations...",
        )
        .await;

        let preserved = self.preserved_state.read().await;
        if let Some(state) = preserved.as_ref() {
            if !state.pending_operations.is_empty() {
                // Send operations for reconciliation
                // let ops: Vec<PendingOperation> = state.pending_operations.iter().map(|op| op.into()).collect();
                // let results = client.reconcile_operations(cube_id, ops).await?;
                // Handle results
            }
        }
    }

    /// Updates recovery progress.
    async fn update_progress(
        &self,
        stage: RecoveryStage,
        progress: u32,
        total: u32,
        message: &str,
    ) {
        *self.recovery_progress.write().await = Some(RecoveryProgress {
            stage,
            progress,
            total,
            message: message.to_string(),
        });
    }

    /// Adds a pending operation to the queue.
    pub async fn queue_pending_operation(&self, operation: PendingOperationEntry) {
        let mut preserved = self.preserved_state.write().await;
        if let Some(state) = preserved.as_mut() {
            state.pending_operations.push(operation);
        }
    }

    /// Clears the recovery state after successful recovery.
    pub async fn clear_recovery_state(&self) {
        *self.state.write().await = RecoveryState::Ready;
        *self.preserved_state.write().await = None;
        *self.reconnect_attempt.write().await = 0;
        *self.recovery_progress.write().await = None;
    }

    /// Shuts down the recovery manager.
    pub async fn shutdown(&self) {
        *self.shutdown.write().await = true;
    }
}

impl Clone for RecoveryManager {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            state: self.state.clone(),
            preserved_state: self.preserved_state.clone(),
            client_state: self.client_state.clone(),
            reconnect_attempt: self.reconnect_attempt.clone(),
            last_heartbeat_received: self.last_heartbeat_received.clone(),
            recovery_progress: self.recovery_progress.clone(),
            shutdown: self.shutdown.clone(),
        }
    }
}

/// Extension trait for Client to support recovery operations.
#[async_trait::async_trait]
pub trait RecoveryClient {
    /// Sends a recovery request to the server.
    async fn send_recovery_request(
        &self,
        request_id: RequestId,
        session_id: Option<SessionId>,
        cube_id: CubeId,
        client_sequence: EventSequence,
        workspace_version: Version,
        pending_operation_ids: Vec<RequestId>,
    ) -> Result<RecoveryPlan, String>;

    /// Requests event replay from the server.
    async fn replay_events(
        &self,
        cube_id: CubeId,
        from_sequence: EventSequence,
        to_sequence: Option<EventSequence>,
    ) -> Result<Vec<nexus_core::event::Event>, String>;

    /// Requests a snapshot from the server.
    async fn request_snapshot(
        &self,
        cube_id: CubeId,
        snapshot_id: nexus_core::ids::SnapshotId,
    ) -> Result<nexus_snapshot::model::WorkspaceSnapshot, String>;

    /// Sends pending operations for reconciliation.
    async fn reconcile_operations(
        &self,
        cube_id: CubeId,
        operations: Vec<PendingOperation>,
    ) -> Result<Vec<OperationReconciliationResult>, String>;
}

impl From<&PendingOperationEntry> for PendingOperation {
    fn from(entry: &PendingOperationEntry) -> Self {
        Self {
            operation_id: entry.operation_id,
            file_id: entry.file_id,
            base_version: entry.base_version,
            payload: entry.payload.clone(),
            created_at: entry.created_at,
            retry_count: entry.retry_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::ids::FileId;

    #[tokio::test]
    async fn test_recovery_manager_creation() {
        let client_state = Arc::new(RwLock::new(ClientState::default()));
        let config = RecoveryConfig::default();
        let manager = RecoveryManager::new(config, client_state);

        assert_eq!(manager.state().await, RecoveryState::Ready);
    }

    #[tokio::test]
    async fn test_backoff_calculation() {
        let client_state = Arc::new(RwLock::new(ClientState::default()));
        let config = RecoveryConfig::default();
        let manager = RecoveryManager::new(config.clone(), client_state);

        let delay0 = manager.calculate_backoff(0);
        let delay1 = manager.calculate_backoff(1);
        let delay2 = manager.calculate_backoff(2);

        assert!(delay0 >= config.min_reconnect_delay);
        assert!(delay1 >= delay0);
        assert!(delay2 >= delay1);
    }

    #[tokio::test]
    async fn test_preserve_state() {
        let client_state = Arc::new(RwLock::new(ClientState::default()));
        let config = RecoveryConfig::default();
        let manager = RecoveryManager::new(config, client_state.clone());

        // Add some files to workspace
        {
            let mut state = client_state.write().await;
            state
                .workspace
                .files
                .push((FileId::new(), std::path::PathBuf::from("src/main.rs")));
        }

        manager.preserve_local_state().await;

        let preserved = manager.preserved_state.read().await;
        assert!(preserved.is_some());
        assert_eq!(preserved.as_ref().unwrap().open_files.len(), 1);
    }
}
