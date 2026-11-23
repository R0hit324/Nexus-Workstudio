//! Runtime manager - orchestrates runtime lifecycle, persistence, and coordination.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use nexus_core::{
    CubeId, ExecRequest, ExecResult, HealthStatus, RuntimeId, RuntimeManifest, RuntimeMetadata,
    RuntimeStatus, Timestamp,
};
use nexus_events::bus::EventBus;
use parking_lot::RwLock;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use crate::backend::{BackendFactory, RuntimeBackend};
use crate::ports::PortAllocator;
use crate::processes::ProcessManager;
use nexus_core::ProcessStatus;

/// Configuration for the runtime manager.
#[derive(Clone)]
pub struct RuntimeManagerConfig {
    /// Root directory for runtime data
    pub data_root: PathBuf,
    /// Backend factory
    pub backend: BackendFactory,
    /// Port allocator
    pub port_allocator: PortAllocator,
    /// Process manager
    pub process_manager: ProcessManager,
    /// Event bus for runtime events
    pub event_bus: Arc<EventBus>,
    /// Default runtime manifest template
    pub default_manifest: RuntimeManifest,
    /// Auto-start policy
    pub auto_start_policy: nexus_core::AutoStartPolicy,
    /// Health check interval
    pub health_check_interval: Duration,
    /// Idle timeout before auto-stop
    pub idle_timeout: Duration,
}

/// Runtime state with metadata and backend info.
#[derive(Debug, Clone)]
pub struct RuntimeState {
    pub metadata: RuntimeMetadata,
    pub manifest: RuntimeManifest,
    pub container_id: Option<String>,
    pub assigned_ports: Vec<nexus_core::PortMapping>,
    pub health: HealthStatus,
    pub last_health_check: Option<Timestamp>,
    pub process_count: u32,
}

/// Manages Cube runtimes.
pub struct RuntimeManager {
    config: RuntimeManagerConfig,
    runtimes: Arc<RwLock<HashMap<RuntimeId, RuntimeState>>>,
    cube_to_runtime: Arc<RwLock<HashMap<CubeId, RuntimeId>>>,
    shutdown_tx: broadcast::Sender<()>,
    health_check_task: tokio::sync::RwLock<Option<tokio::task::JoinHandle<()>>>,
}

impl RuntimeManager {
    /// Creates a new runtime manager.
    pub fn new(config: RuntimeManagerConfig) -> Self {
        let (shutdown_tx, _shutdown_rx) = broadcast::channel(16);
        Self {
            config,
            runtimes: Arc::new(RwLock::new(HashMap::new())),
            cube_to_runtime: Arc::new(RwLock::new(HashMap::new())),
            shutdown_tx,
            health_check_task: tokio::sync::RwLock::new(None),
        }
    }

    /// Creates a runtime for a Cube.
    pub async fn create_runtime(
        &self,
        cube_id: CubeId,
        manifest: Option<RuntimeManifest>,
    ) -> Result<RuntimeId, anyhow::Error> {
        let runtime_id = RuntimeId::new();
        let manifest = manifest.unwrap_or_else(|| self.default_manifest_for_cube(cube_id));

        // Allocate ports
        let mut manifest = manifest;
        let backend_manifest = manifest.clone();
        for port in &mut manifest.ports {
            if port.host_port == 0 {
                port.host_port = self
                    .config
                    .port_allocator
                    .allocate(cube_id, port.container_port)
                    .await?;
            }
        }

        // Create runtime via backend
        let backend = self.get_backend();
        backend.create(runtime_id, cube_id, manifest).await?;

        // Create metadata
        let metadata = RuntimeMetadata {
            runtime_id,
            cube_id,
            backend: backend_manifest.backend.clone(),
            manifest: backend_manifest.clone(),
            status: RuntimeStatus::Created,
            created_at: Timestamp::now(),
            started_at: None,
            stopped_at: None,
            assigned_ports: backend_manifest.ports.clone(),
            container_id: None,
        };

        // Store state
        let state = RuntimeState {
            metadata: metadata.clone(),
            manifest: backend_manifest.clone(),
            container_id: None,
            assigned_ports: backend_manifest.ports.clone(),
            health: HealthStatus::Healthy,
            last_health_check: None,
            process_count: 0,
        };

        self.runtimes.write().insert(runtime_id, state);
        self.cube_to_runtime.write().insert(cube_id, runtime_id);

        // Emit event
        self.emit_event(crate::events::RuntimeEvent::Created {
            runtime_id,
            cube_id,
        })
        .await;

        info!(%runtime_id, %cube_id, "Runtime created");
        Ok(runtime_id)
    }

    /// Starts a runtime.
    pub async fn start_runtime(&self, runtime_id: RuntimeId) -> Result<(), anyhow::Error> {
        let mut runtimes = self.runtimes.write();
        let state = runtimes
            .get_mut(&runtime_id)
            .ok_or_else(|| anyhow::anyhow!("Runtime not found"))?;

        if state.metadata.status == RuntimeStatus::Running {
            return Ok(());
        }

        state.metadata.status = RuntimeStatus::Starting;

        let backend = self.get_backend();
        backend.start(runtime_id).await?;

        state.metadata.status = RuntimeStatus::Running;
        state.metadata.started_at = Some(Timestamp::now());
        state.health = HealthStatus::Healthy;
        state.last_health_check = Some(Timestamp::now());

        self.emit_event(crate::events::RuntimeEvent::Started { runtime_id })
            .await;

        info!(%runtime_id, "Runtime started");
        Ok(())
    }

    /// Stops a runtime.
    pub async fn stop_runtime(
        &self,
        runtime_id: RuntimeId,
        timeout: Duration,
    ) -> Result<(), anyhow::Error> {
        let mut runtimes = self.runtimes.write();
        let state = runtimes
            .get_mut(&runtime_id)
            .ok_or_else(|| anyhow::anyhow!("Runtime not found"))?;

        if state.metadata.status == RuntimeStatus::Stopped {
            return Ok(());
        }

        state.metadata.status = RuntimeStatus::Stopping;

        let backend = self.get_backend();
        backend.stop(runtime_id, timeout).await?;

        state.metadata.status = RuntimeStatus::Stopped;
        state.metadata.stopped_at = Some(Timestamp::now());
        state.process_count = 0;

        self.emit_event(crate::events::RuntimeEvent::Stopped { runtime_id })
            .await;

        info!(%runtime_id, "Runtime stopped");
        Ok(())
    }

    /// Restarts a runtime.
    pub async fn restart_runtime(&self, runtime_id: RuntimeId) -> Result<(), anyhow::Error> {
        let mut runtimes = self.runtimes.write();
        let state = runtimes
            .get_mut(&runtime_id)
            .ok_or_else(|| anyhow::anyhow!("Runtime not found"))?;

        state.metadata.status = RuntimeStatus::Restarting;

        let backend = self.get_backend();
        backend.restart(runtime_id).await?;

        state.metadata.status = RuntimeStatus::Running;
        state.metadata.started_at = Some(Timestamp::now());
        state.metadata.stopped_at = None;

        self.emit_event(crate::events::RuntimeEvent::Restarted { runtime_id })
            .await;

        info!(%runtime_id, "Runtime restarted");
        Ok(())
    }

    /// Removes a runtime.
    pub async fn remove_runtime(&self, runtime_id: RuntimeId) -> Result<(), anyhow::Error> {
        let mut runtimes = self.runtimes.write();
        let state = runtimes
            .remove(&runtime_id)
            .ok_or_else(|| anyhow::anyhow!("Runtime not found"))?;

        // Release ports
        for port in &state.assigned_ports {
            self.config
                .port_allocator
                .release(state.metadata.cube_id, port.container_port)
                .await?;
        }

        // Remove cube mapping
        self.cube_to_runtime.write().remove(&state.metadata.cube_id);

        let backend = self.get_backend();
        backend.remove(runtime_id).await?;

        self.emit_event(crate::events::RuntimeEvent::Removed { runtime_id })
            .await;

        info!(%runtime_id, "Runtime removed");
        Ok(())
    }

    /// Gets the runtime ID for a cube.
    pub fn get_runtime_id(&self, cube_id: CubeId) -> Option<RuntimeId> {
        self.cube_to_runtime.read().get(&cube_id).copied()
    }

    /// Gets runtime metadata.
    pub fn get_runtime(&self, runtime_id: RuntimeId) -> Option<RuntimeMetadata> {
        self.runtimes
            .read()
            .get(&runtime_id)
            .map(|s| s.metadata.clone())
    }

    /// Gets all runtimes for a cube.
    pub fn get_runtimes_for_cube(&self, cube_id: CubeId) -> Vec<RuntimeMetadata> {
        self.runtimes
            .read()
            .values()
            .filter(|s| s.metadata.cube_id == cube_id)
            .map(|s| s.metadata.clone())
            .collect()
    }

    /// Executes a command in a runtime.
    pub async fn exec(&self, request: ExecRequest) -> Result<ExecResult, anyhow::Error> {
        // Validate runtime exists
        let runtime_id = request.runtime_id;
        let command = request.command.clone();
        let args = request.args.clone();
        let runtimes = self.runtimes.read();
        let state = runtimes
            .get(&runtime_id)
            .ok_or_else(|| anyhow::anyhow!("Runtime not found"))?;

        if state.metadata.status != RuntimeStatus::Running {
            return Err(anyhow::anyhow!("Runtime not running"));
        }

        let backend = self.get_backend();
        let result = backend.exec(request).await?;

        // Track process
        if let Err(err) = self
            .config
            .process_manager
            .track_process(crate::processes::TrackedProcess {
                process_id: result.process_id,
                runtime_id,
                cube_id: state.metadata.cube_id,
                command: format!("{} {}", command, args.join(" ")),
                started_at: Timestamp::now(),
                status: if result.exit_code.is_some() {
                    ProcessStatus::Exited
                } else {
                    ProcessStatus::Running
                },
            })
            .await
        {
            warn!(runtime_id = %runtime_id, error = %err, "failed to track exec process");
        }

        Ok(result)
    }

    /// Gets runtime status.
    pub async fn status(&self, runtime_id: RuntimeId) -> Result<RuntimeMetadata, anyhow::Error> {
        let backend = self.get_backend();
        backend.inspect(runtime_id).await
    }

    /// Performs health check on a runtime.
    pub async fn health_check(&self, runtime_id: RuntimeId) -> Result<HealthStatus, anyhow::Error> {
        let mut runtimes = self.runtimes.write();
        let state = runtimes
            .get_mut(&runtime_id)
            .ok_or_else(|| anyhow::anyhow!("Runtime not found"))?;

        let backend = self.get_backend();
        let health = backend.health_check(runtime_id).await?;

        state.health = health;
        state.last_health_check = Some(Timestamp::now());

        if health == HealthStatus::Unhealthy {
            state.metadata.status = RuntimeStatus::Failed;
            self.emit_event(crate::events::RuntimeEvent::HealthFailed { runtime_id })
                .await;
        }

        Ok(health)
    }

    /// Starts the health check loop.
    pub async fn start_health_checks(&self) {
        let runtimes = self.runtimes.clone();
        let config = self.config.clone();
        let backend_factory = self.config.backend.clone();
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(config.health_check_interval);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let backend = backend_factory.as_backend();
                        let runtime_ids: Vec<RuntimeId> = runtimes.read().keys().copied().collect();
                        for id in runtime_ids {
                            let _ = backend.health_check(id).await;
                        }
                    }
                    _ = shutdown_rx.recv() => break,
                }
            }
        });

        *self.health_check_task.write().await = Some(handle);
    }

    /// Stops the health check loop.
    pub async fn stop_health_checks(&self) {
        let _ = self.shutdown_tx.send(());
        if let Some(handle) = self.health_check_task.write().await.take() {
            handle.abort();
        }
    }

    /// Starts idle timeout monitoring.
    pub async fn start_idle_monitor(&self) {
        let runtimes = self.runtimes.clone();
        let config = self.config.clone();
        let backend_factory = self.config.backend.clone();
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let backend = backend_factory.as_backend();
                        let now = Timestamp::now();
                        let runtime_ids: Vec<RuntimeId> = runtimes.read().keys().copied().collect();
                        for id in runtime_ids {
                            let should_stop = {
                                let state = runtimes.read().get(&id).cloned();
                                match state {
                                    Some(s) if s.metadata.status == RuntimeStatus::Running => {
                                        let idle = now.as_millis().saturating_sub(
                                            s.last_health_check.unwrap_or(s.metadata.started_at.unwrap_or(s.metadata.created_at)).as_millis()
                                        );
                                        (idle as u128) > config.idle_timeout.as_millis()
                                    }
                                    _ => false,
                                }
                            };
                            if should_stop {
                                let _ = backend.stop(id, Duration::from_secs(30)).await;
                            }
                        }
                    }
                    _ = shutdown_rx.recv() => break,
                }
            }
        });
    }

    /// Creates a default manifest for a cube.
    fn default_manifest_for_cube(&self, _cube_id: CubeId) -> RuntimeManifest {
        self.config.default_manifest.clone()
    }

    /// Gets the appropriate backend.
    fn get_backend(&self) -> &dyn RuntimeBackend {
        self.config.backend.as_backend()
    }

    /// Emits a runtime event.
    async fn emit_event(&self, event: crate::events::RuntimeEvent) {
        // In real implementation, would publish to event bus
        debug!(event = ?event, "Runtime event emitted");
    }
}

/// Events module for runtime.
pub mod events {
    use nexus_core::{CubeId, RuntimeId};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum RuntimeEvent {
        Created {
            runtime_id: RuntimeId,
            cube_id: CubeId,
        },
        Starting {
            runtime_id: RuntimeId,
        },
        Started {
            runtime_id: RuntimeId,
        },
        Stopping {
            runtime_id: RuntimeId,
        },
        Stopped {
            runtime_id: RuntimeId,
        },
        Restarting {
            runtime_id: RuntimeId,
        },
        Restarted {
            runtime_id: RuntimeId,
        },
        Failed {
            runtime_id: RuntimeId,
            reason: String,
        },
        HealthFailed {
            runtime_id: RuntimeId,
        },
        Removed {
            runtime_id: RuntimeId,
        },
    }
}

impl Clone for RuntimeManager {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            runtimes: self.runtimes.clone(),
            cube_to_runtime: self.cube_to_runtime.clone(),
            shutdown_tx: self.shutdown_tx.clone(),
            health_check_task: tokio::sync::RwLock::new(None),
        }
    }
}
