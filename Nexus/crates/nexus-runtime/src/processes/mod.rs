//! Process manager for runtime processes.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use nexus_core::{CubeId, ProcessStatus, RuntimeId, Timestamp};
use parking_lot::RwLock;
use tokio::sync::broadcast;
use tracing::debug;

/// Configuration for process manager.
#[derive(Debug, Clone)]
pub struct ProcessManagerConfig {
    /// Maximum processes per runtime
    pub max_processes_per_runtime: u32,
    /// Process cleanup interval
    pub cleanup_interval: Duration,
    /// Process TTL after exit
    pub exited_ttl: Duration,
}

impl Default for ProcessManagerConfig {
    fn default() -> Self {
        Self {
            max_processes_per_runtime: 100,
            cleanup_interval: Duration::from_secs(60),
            exited_ttl: Duration::from_secs(300),
        }
    }
}

/// A process being tracked.
#[derive(Debug, Clone)]
pub struct TrackedProcess {
    pub process_id: u32,
    pub runtime_id: RuntimeId,
    pub cube_id: CubeId,
    pub command: String,
    pub started_at: Timestamp,
    pub status: ProcessStatus,
}

/// Manages processes within runtimes.
pub struct ProcessManager {
    config: ProcessManagerConfig,
    processes: Arc<RwLock<HashMap<u32, TrackedProcess>>>,
    runtime_processes: Arc<RwLock<HashMap<RuntimeId, Vec<u32>>>>,
    shutdown_tx: broadcast::Sender<()>,
}

impl ProcessManager {
    /// Creates a new process manager.
    pub fn new(config: ProcessManagerConfig) -> Self {
        let (shutdown_tx, _shutdown_rx) = broadcast::channel(16);
        Self {
            config,
            processes: Arc::new(RwLock::new(HashMap::new())),
            runtime_processes: Arc::new(RwLock::new(HashMap::new())),
            shutdown_tx,
        }
    }

    /// Tracks a new process.
    pub async fn track_process(&self, process: TrackedProcess) -> Result<(), anyhow::Error> {
        let mut processes = self.processes.write();
        let mut runtime_procs = self.runtime_processes.write();

        // Check limit
        let count = runtime_procs
            .get(&process.runtime_id)
            .map(|v| v.len())
            .unwrap_or(0);
        if count >= self.config.max_processes_per_runtime as usize {
            return Err(anyhow::anyhow!("Max processes per runtime exceeded"));
        }

        processes.insert(process.process_id, process.clone());
        runtime_procs
            .entry(process.runtime_id)
            .or_default()
            .push(process.process_id);

        debug!(process_id = process.process_id, runtime_id = %process.runtime_id, "Process tracked");
        Ok(())
    }

    /// Updates process status.
    pub async fn update_status(&self, process_id: u32, status: ProcessStatus) {
        if let Some(process) = self.processes.write().get_mut(&process_id) {
            process.status = status;
        }
    }

    /// Sets process exit code.
    pub async fn set_exit_code(&self, process_id: u32, exit_code: i32) {
        if let Some(process) = self.processes.write().get_mut(&process_id) {
            process.status = if exit_code == 0 {
                ProcessStatus::Exited
            } else {
                ProcessStatus::Failed
            };
        }
    }

    /// Gets a process by ID.
    pub fn get_process(&self, process_id: u32) -> Option<TrackedProcess> {
        self.processes.read().get(&process_id).cloned()
    }

    /// Gets all processes for a runtime.
    pub fn get_runtime_processes(&self, runtime_id: RuntimeId) -> Vec<TrackedProcess> {
        let runtime_procs = self.runtime_processes.read();
        let processes = self.processes.read();

        runtime_procs
            .get(&runtime_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| processes.get(id).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Gets all processes for a cube.
    pub fn get_cube_processes(&self, cube_id: nexus_core::CubeId) -> Vec<TrackedProcess> {
        let processes = self.processes.read();
        processes
            .values()
            .filter(|p| p.cube_id == cube_id)
            .cloned()
            .collect()
    }

    /// Removes a process from tracking.
    pub async fn remove_process(&self, process_id: u32) {
        if let Some(process) = self.processes.write().remove(&process_id) {
            if let Some(procs) = self.runtime_processes.write().get_mut(&process.runtime_id) {
                procs.retain(|&id| id != process_id);
            }
        }
    }

    /// Cleans up exited processes older than TTL.
    pub async fn cleanup(&self) {
        let now = Timestamp::now();
        let ttl_millis = self.config.exited_ttl.as_millis() as u64;

        let to_remove: Vec<u32> = {
            let processes = self.processes.read();
            processes
                .iter()
                .filter(|(_, p)| {
                    matches!(
                        p.status,
                        ProcessStatus::Exited | ProcessStatus::Failed | ProcessStatus::Killed
                    ) && now.as_millis().saturating_sub(p.started_at.as_millis()) > ttl_millis
                })
                .map(|(id, _)| *id)
                .collect()
        };

        for id in to_remove {
            self.remove_process(id).await;
        }
    }

    /// Starts the cleanup task.
    pub async fn start_cleanup_task(&self) {
        let processes = self.processes.clone();
        let _runtime_processes = self.runtime_processes.clone();
        let config = self.config.clone();
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(config.cleanup_interval);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let now = Timestamp::now();
                        let ttl_millis = config.exited_ttl.as_millis() as u64;

                        let to_remove: Vec<u32> = {
                            let procs = processes.read();
                            procs.iter()
                                .filter(|(_, p)| {
                                    matches!(p.status, ProcessStatus::Exited | ProcessStatus::Failed | ProcessStatus::Killed)
                                        && now.as_millis().saturating_sub(p.started_at.as_millis()) > ttl_millis
                                })
                                .map(|(id, _)| *id)
                                .collect()
                        };

                        for id in to_remove {
                            processes.write().remove(&id);
                        }
                    }
                    _ = shutdown_rx.recv() => break,
                }
            }
        });
    }
}

impl Default for ProcessManager {
    fn default() -> Self {
        Self::new(ProcessManagerConfig::default())
    }
}

impl Clone for ProcessManager {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            processes: self.processes.clone(),
            runtime_processes: self.runtime_processes.clone(),
            shutdown_tx: self.shutdown_tx.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::{CubeId, RuntimeId};

    #[tokio::test]
    async fn test_process_tracking() {
        let manager = ProcessManager::default();
        let runtime_id = RuntimeId::new();
        let cube_id = CubeId::new();

        manager
            .track_process(TrackedProcess {
                process_id: 123,
                runtime_id,
                cube_id,
                command: "cargo build".into(),
                started_at: Timestamp::now(),
                status: ProcessStatus::Starting,
            })
            .await
            .unwrap();

        let procs = manager.get_runtime_processes(runtime_id);
        assert_eq!(procs.len(), 1);
        assert_eq!(procs[0].process_id, 123);

        manager.update_status(123, ProcessStatus::Running).await;
        let proc = manager.get_process(123).unwrap();
        assert_eq!(proc.status, ProcessStatus::Running);
    }
}
