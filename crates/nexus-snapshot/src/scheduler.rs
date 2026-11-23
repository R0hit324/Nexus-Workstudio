use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use nexus_core::event::EventSequence;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::time::interval;
use tracing::debug;

use crate::model::SnapshotType;

/// Snapshot trigger conditions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotTrigger {
    /// Snapshot after N durable events.
    EventCount(EventSequence),
    /// Snapshot after time interval.
    TimeInterval(Duration),
    /// Snapshot when workspace size exceeds threshold.
    SizeThreshold(u64),
    /// Manual trigger.
    Manual,
    /// Combined trigger (any condition met).
    Any(Box<SnapshotTrigger>, Box<SnapshotTrigger>),
}

impl SnapshotTrigger {
    /// Creates an event count trigger.
    pub fn event_count(count: u64) -> Self {
        Self::EventCount(EventSequence::new(count))
    }

    /// Creates a time interval trigger.
    pub fn time_interval(duration: Duration) -> Self {
        Self::TimeInterval(duration)
    }

    /// Creates a size threshold trigger.
    pub fn size_threshold(bytes: u64) -> Self {
        Self::SizeThreshold(bytes)
    }

    /// Creates a manual trigger.
    pub fn manual() -> Self {
        Self::Manual
    }

    /// Combines two triggers with OR logic.
    pub fn or(self, other: Self) -> Self {
        Self::Any(Box::new(self), Box::new(other))
    }
}

/// Snapshot scheduling policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotPolicy {
    /// Triggers for snapshot creation.
    pub triggers: Vec<SnapshotTrigger>,
    /// Minimum interval between snapshots.
    pub min_interval: Duration,
    /// Whether to create incremental snapshots.
    pub use_incremental: bool,
    /// Maximum incremental chain length before forcing full snapshot.
    pub max_incremental_chain: usize,
    /// Base snapshot interval for incremental snapshots.
    pub incremental_base_interval: nexus_core::event::EventSequence,
}

impl Default for SnapshotPolicy {
    fn default() -> Self {
        Self {
            triggers: vec![
                SnapshotTrigger::event_count(10000),
                SnapshotTrigger::time_interval(Duration::from_secs(3600)), // 1 hour
            ],
            min_interval: Duration::from_secs(300), // 5 minutes
            use_incremental: true,
            max_incremental_chain: 10,
            incremental_base_interval: nexus_core::event::EventSequence::new(50000),
        }
    }
}

/// Snapshot info for tracking.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct SnapshotInfo {
    event_sequence: nexus_core::event::EventSequence,
    timestamp: SystemTime,
    snapshot_type: SnapshotType,
    snapshot_id: nexus_core::ids::SnapshotId,
}

/// Snapshot scheduler for automatic snapshot creation.
pub struct SnapshotScheduler {
    policy: SnapshotPolicy,
    last_snapshot: Arc<RwLock<HashMap<nexus_core::ids::CubeId, SnapshotInfo>>>,
    incremental_chain_length: Arc<RwLock<HashMap<nexus_core::ids::CubeId, usize>>>,
    retention_manager: Arc<crate::retention::RetentionManager>,
    store: Arc<dyn crate::store::SnapshotStore>,
    index: Arc<crate::metadata::SnapshotIndex>,
}

impl SnapshotScheduler {
    pub fn new(
        policy: SnapshotPolicy,
        store: Arc<dyn crate::store::SnapshotStore>,
        retention_manager: Arc<crate::retention::RetentionManager>,
        index: Arc<crate::metadata::SnapshotIndex>,
    ) -> Self {
        Self {
            policy,
            last_snapshot: Arc::new(RwLock::new(HashMap::new())),
            incremental_chain_length: Arc::new(RwLock::new(HashMap::new())),
            retention_manager,
            store,
            index,
        }
    }

    /// Checks if a snapshot should be created for a cube.
    pub fn should_snapshot(
        &self,
        cube_id: nexus_core::ids::CubeId,
        current_sequence: nexus_core::event::EventSequence,
        workspace_size: u64,
    ) -> (bool, SnapshotType) {
        let last = self.last_snapshot.read().get(&cube_id).cloned();
        let incremental_chain = self
            .incremental_chain_length
            .read()
            .get(&cube_id)
            .copied()
            .unwrap_or(0);

        // Check minimum interval
        if let Some(last) = last {
            if last.timestamp.elapsed().unwrap_or_default() < self.policy.min_interval {
                return (false, SnapshotType::Full);
            }
        }

        // Check triggers
        for trigger in &self.policy.triggers {
            let (should, snap_type) = self.check_trigger(
                trigger.clone(),
                cube_id,
                current_sequence,
                workspace_size,
                incremental_chain,
            );
            if should {
                return (true, snap_type);
            }
        }

        (false, SnapshotType::Full)
    }

    fn check_trigger(
        &self,
        trigger: SnapshotTrigger,
        cube_id: nexus_core::ids::CubeId,
        current_sequence: nexus_core::event::EventSequence,
        workspace_size: u64,
        incremental_chain: usize,
    ) -> (bool, SnapshotType) {
        let last = self.last_snapshot.read().get(&cube_id).cloned();

        match trigger {
            SnapshotTrigger::EventCount(count) => {
                if let Some(last) = last {
                    if current_sequence
                        .value()
                        .saturating_sub(last.event_sequence.value())
                        >= count.value()
                    {
                        return (true, self.determine_snapshot_type(incremental_chain));
                    }
                } else if current_sequence.value() >= count.value() {
                    return (true, SnapshotType::Full);
                }
            }
            SnapshotTrigger::TimeInterval(interval) => {
                if let Some(last) = last {
                    if last.timestamp.elapsed().unwrap_or_default() >= interval {
                        return (true, self.determine_snapshot_type(incremental_chain));
                    }
                }
            }
            SnapshotTrigger::SizeThreshold(threshold) => {
                if workspace_size >= threshold {
                    return (true, self.determine_snapshot_type(incremental_chain));
                }
            }
            SnapshotTrigger::Manual => {
                return (true, self.determine_snapshot_type(incremental_chain));
            }
            SnapshotTrigger::Any(a, b) => {
                let (a_result, a_type) = self.check_trigger(
                    *a,
                    cube_id,
                    current_sequence,
                    workspace_size,
                    incremental_chain,
                );
                if a_result {
                    return (a_result, a_type);
                }
                let (b_result, b_type) = self.check_trigger(
                    *b,
                    cube_id,
                    current_sequence,
                    workspace_size,
                    incremental_chain,
                );
                if b_result {
                    return (b_result, b_type);
                }
            }
        }
        (false, SnapshotType::Full)
    }

    fn determine_snapshot_type(&self, incremental_chain: usize) -> SnapshotType {
        if !self.policy.use_incremental || incremental_chain >= self.policy.max_incremental_chain {
            SnapshotType::Full
        } else {
            SnapshotType::Incremental {
                base_snapshot_id: nexus_core::ids::SnapshotId::default(),
            }
        }
    }

    /// Records a successful snapshot creation.
    pub fn record_snapshot(
        &self,
        cube_id: nexus_core::ids::CubeId,
        snapshot: &crate::model::WorkspaceSnapshot,
    ) {
        let mut last = self.last_snapshot.write();
        last.insert(
            cube_id,
            SnapshotInfo {
                event_sequence: snapshot.event_sequence,
                timestamp: SystemTime::now(),
                snapshot_type: snapshot.snapshot_type,
                snapshot_id: snapshot.snapshot_id,
            },
        );

        if snapshot.is_incremental() {
            let mut chain = self.incremental_chain_length.write();
            let count = chain.get(&cube_id).copied().unwrap_or(0);
            chain.insert(cube_id, count + 1);
        } else {
            self.incremental_chain_length.write().insert(cube_id, 0);
        }
    }

    /// Runs the scheduler loop.
    pub async fn run(&self, mut rx: tokio::sync::mpsc::Receiver<SchedulerCommand>) {
        let mut interval = interval(Duration::from_secs(60)); // Check every minute

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    self.periodic_check().await;
                }
                cmd = rx.recv() => {
                    match cmd {
                        Some(SchedulerCommand::TriggerSnapshot(cube_id, seq, size)) => {
                            if let Some(sender) = self.trigger_manual(cube_id, seq, size).await {
                                let _ = sender.send(());
                            }
                        }
                        Some(SchedulerCommand::Shutdown) => break,
                        None => break,
                    }
                }
            }
        }
    }

    async fn periodic_check(&self) {
        // This would check all active cubes
        // In practice, would get active cubes from CubeManager
        debug!("Running periodic snapshot check");
    }

    async fn trigger_manual(
        &self,
        _cube_id: nexus_core::ids::CubeId,
        _seq: nexus_core::event::EventSequence,
        _size: u64,
    ) -> Option<tokio::sync::oneshot::Sender<()>> {
        None
    }

    /// Applies retention policy.
    pub async fn apply_retention(
        &self,
    ) -> Result<crate::retention::RetentionResult, crate::store::StoreError> {
        let all_snapshots: Vec<crate::metadata::SnapshotMetadata> = self.index.values();
        let result = self.retention_manager.apply(&all_snapshots);

        // Delete old snapshots
        for snapshot_id in &result.deleted {
            self.store.delete(*snapshot_id).await?;
        }

        Ok(result)
    }
}

/// Commands for the scheduler.
pub enum SchedulerCommand {
    TriggerSnapshot(
        nexus_core::ids::CubeId,
        nexus_core::event::EventSequence,
        u64,
    ),
    Shutdown,
}

/// Scheduler handle for external control.
#[derive(Clone)]
pub struct SchedulerHandle {
    tx: tokio::sync::mpsc::Sender<SchedulerCommand>,
}

impl SchedulerHandle {
    pub fn new(tx: tokio::sync::mpsc::Sender<SchedulerCommand>) -> Self {
        Self { tx }
    }

    pub async fn trigger_snapshot(
        &self,
        cube_id: nexus_core::ids::CubeId,
        sequence: nexus_core::event::EventSequence,
        size: u64,
    ) -> Result<(), tokio::sync::mpsc::error::SendError<SchedulerCommand>> {
        self.tx
            .send(SchedulerCommand::TriggerSnapshot(cube_id, sequence, size))
            .await
    }

    pub async fn shutdown(
        &self,
    ) -> Result<(), tokio::sync::mpsc::error::SendError<SchedulerCommand>> {
        self.tx.send(SchedulerCommand::Shutdown).await
    }
}

impl SnapshotScheduler {
    pub fn handle(&self) -> SchedulerHandle {
        let (tx, _rx) = tokio::sync::mpsc::channel(32);
        // Note: In real implementation, would spawn the run() task
        SchedulerHandle::new(tx)
    }
}
