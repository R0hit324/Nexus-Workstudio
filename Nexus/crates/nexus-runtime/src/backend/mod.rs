//! Runtime backend abstraction.
//!
//! Provides a trait for different runtime backends (Docker, local process, etc.)

use std::time::Duration;

use async_trait::async_trait;
use nexus_core::{
    CubeId, ExecRequest, ExecResult, HealthStatus, ResourceLimits, RuntimeId, RuntimeManifest,
    RuntimeMetadata, RuntimeStatus,
};
use serde::{Deserialize, Serialize};
use tracing::info;

/// Trait for runtime backends.
///
/// Implementations should provide isolated execution environments for Cubes.
#[async_trait::async_trait]
pub trait RuntimeBackend: Send + Sync {
    /// Creates a new runtime instance for a Cube.
    async fn create(
        &self,
        runtime_id: RuntimeId,
        cube_id: CubeId,
        manifest: RuntimeManifest,
    ) -> Result<(), anyhow::Error>;

    /// Starts an existing runtime.
    async fn start(&self, runtime_id: RuntimeId) -> Result<(), anyhow::Error>;

    /// Stops a running runtime.
    async fn stop(&self, runtime_id: RuntimeId, timeout: Duration) -> Result<(), anyhow::Error>;

    /// Restarts a runtime.
    async fn restart(&self, runtime_id: RuntimeId) -> Result<(), anyhow::Error>;

    /// Removes a runtime completely.
    async fn remove(&self, runtime_id: RuntimeId) -> Result<(), anyhow::Error>;

    /// Executes a command in the runtime.
    async fn exec(&self, request: ExecRequest) -> Result<ExecResult, anyhow::Error>;

    /// Inspects runtime metadata.
    async fn inspect(&self, runtime_id: RuntimeId) -> Result<RuntimeMetadata, anyhow::Error>;

    /// Performs a health check on the runtime.
    async fn health_check(&self, runtime_id: RuntimeId) -> Result<HealthStatus, anyhow::Error>;
}

/// Docker backend configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerConfig {
    /// Docker socket path or URL
    pub socket: String,
    /// Default image if not specified in manifest
    pub default_image: String,
    /// Request timeout
    pub timeout: Duration,
    /// Container name prefix
    pub name_prefix: String,
    /// Network name for containers
    pub network: Option<String>,
    /// Whether to remove containers on stop
    pub remove_on_stop: bool,
    /// Default resource limits
    pub default_limits: ResourceLimits,
}

impl Default for DockerConfig {
    fn default() -> Self {
        Self {
            socket: "unix:///var/run/docker.sock".into(),
            default_image: "ubuntu:22.04".into(),
            timeout: Duration::from_secs(30),
            name_prefix: "nexus-runtime-".into(),
            network: None,
            remove_on_stop: false,
            default_limits: ResourceLimits::default(),
        }
    }
}

/// Docker backend implementation (simplified - requires docker-api feature).
#[derive(Debug, Clone)]
pub struct DockerBackend {
    #[allow(dead_code)]
    config: DockerConfig,
}

impl DockerBackend {
    /// Creates a new Docker backend.
    pub fn new(config: DockerConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl RuntimeBackend for DockerBackend {
    async fn create(
        &self,
        runtime_id: RuntimeId,
        cube_id: CubeId,
        manifest: RuntimeManifest,
    ) -> Result<(), anyhow::Error> {
        info!(%runtime_id, %cube_id, "Creating Docker container (simulated)");
        // In real implementation with docker-api feature:
        // let docker = DockerBuilder::new(&self.config.socket).timeout(self.config.timeout).build()?;
        // docker.containers().create(...).await?;
        let _ = manifest;
        Ok(())
    }

    async fn start(&self, runtime_id: RuntimeId) -> Result<(), anyhow::Error> {
        info!(%runtime_id, "Starting Docker container (simulated)");
        Ok(())
    }

    async fn stop(&self, runtime_id: RuntimeId, _timeout: Duration) -> Result<(), anyhow::Error> {
        info!(%runtime_id, "Stopping Docker container (simulated)");
        Ok(())
    }

    async fn restart(&self, runtime_id: RuntimeId) -> Result<(), anyhow::Error> {
        info!(%runtime_id, "Restarting Docker container (simulated)");
        self.stop(runtime_id, Duration::from_secs(10)).await?;
        self.start(runtime_id).await
    }

    async fn remove(&self, runtime_id: RuntimeId) -> Result<(), anyhow::Error> {
        info!(%runtime_id, "Removing Docker container (simulated)");
        Ok(())
    }

    async fn exec(&self, request: ExecRequest) -> Result<ExecResult, anyhow::Error> {
        info!(runtime_id = %request.runtime_id, command = %request.command, "Executing in container (simulated)");
        Ok(ExecResult {
            process_id: 0,
            exit_code: Some(0),
            stdout: "Simulated output".into(),
            stderr: "".into(),
        })
    }

    async fn inspect(&self, runtime_id: RuntimeId) -> Result<RuntimeMetadata, anyhow::Error> {
        Ok(RuntimeMetadata {
            runtime_id,
            cube_id: CubeId::new(),
            backend: nexus_core::RuntimeBackend::Docker,
            manifest: RuntimeManifest::default(),
            status: RuntimeStatus::Created,
            created_at: nexus_core::Timestamp::now(),
            started_at: None,
            stopped_at: None,
            assigned_ports: Vec::new(),
            container_id: None,
        })
    }

    async fn health_check(&self, _runtime_id: RuntimeId) -> Result<HealthStatus, anyhow::Error> {
        Ok(HealthStatus::Healthy)
    }
}

/// Local process backend (for development/testing).
#[derive(Debug, Clone)]
pub struct LocalProcessBackend {
    config: LocalProcessConfig,
}

#[derive(Debug, Clone, Default)]
pub struct LocalProcessConfig {
    pub workspace_root: std::path::PathBuf,
}

impl LocalProcessBackend {
    pub fn new(config: LocalProcessConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl RuntimeBackend for LocalProcessBackend {
    async fn create(
        &self,
        runtime_id: RuntimeId,
        cube_id: CubeId,
        _manifest: RuntimeManifest,
    ) -> Result<(), anyhow::Error> {
        info!(%runtime_id, %cube_id, "Creating local process runtime");
        Ok(())
    }

    async fn start(&self, runtime_id: RuntimeId) -> Result<(), anyhow::Error> {
        info!(%runtime_id, "Starting local process runtime");
        Ok(())
    }

    async fn stop(&self, runtime_id: RuntimeId, _timeout: Duration) -> Result<(), anyhow::Error> {
        info!(%runtime_id, "Stopping local process runtime");
        Ok(())
    }

    async fn restart(&self, runtime_id: RuntimeId) -> Result<(), anyhow::Error> {
        self.stop(runtime_id, Duration::from_secs(10)).await?;
        self.start(runtime_id).await
    }

    async fn remove(&self, runtime_id: RuntimeId) -> Result<(), anyhow::Error> {
        info!(%runtime_id, "Removing local process runtime");
        Ok(())
    }

    async fn exec(&self, request: ExecRequest) -> Result<ExecResult, anyhow::Error> {
        use tokio::process::Command;

        let mut cmd = Command::new(&request.command);
        cmd.args(&request.args);
        cmd.envs(request.env.iter().map(|v| (&v.key, &v.value)));

        if let Some(dir) = request.working_dir {
            cmd.current_dir(dir);
        } else {
            cmd.current_dir(&self.config.workspace_root);
        }

        let output = cmd.output().await?;

        Ok(ExecResult {
            process_id: output.status.code().unwrap_or(-1) as u32,
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into(),
            stderr: String::from_utf8_lossy(&output.stderr).into(),
        })
    }

    async fn inspect(&self, runtime_id: RuntimeId) -> Result<RuntimeMetadata, anyhow::Error> {
        Ok(RuntimeMetadata {
            runtime_id,
            cube_id: CubeId::new(),
            backend: nexus_core::RuntimeBackend::LocalProcess,
            manifest: RuntimeManifest::default(),
            status: RuntimeStatus::Running,
            created_at: nexus_core::Timestamp::now(),
            started_at: Some(nexus_core::Timestamp::now()),
            stopped_at: None,
            assigned_ports: Vec::new(),
            container_id: None,
        })
    }

    async fn health_check(&self, _runtime_id: RuntimeId) -> Result<HealthStatus, anyhow::Error> {
        Ok(HealthStatus::Healthy)
    }
}

/// Backend factory for creating runtime backends.
#[derive(Debug, Clone)]
pub enum BackendFactory {
    Docker(DockerBackend),
    LocalProcess(LocalProcessBackend),
}

impl BackendFactory {
    /// Creates a Docker backend.
    pub fn docker(config: DockerConfig) -> Self {
        Self::Docker(DockerBackend::new(config))
    }

    /// Creates a local process backend.
    pub fn local_process(config: LocalProcessConfig) -> Self {
        Self::LocalProcess(LocalProcessBackend::new(config))
    }

    /// Returns a boxed trait object for the backend.
    pub fn into_backend(self) -> Box<dyn RuntimeBackend> {
        match self {
            Self::Docker(b) => Box::new(b),
            Self::LocalProcess(b) => Box::new(b),
        }
    }

    /// Returns a reference to the backend as a trait object.
    pub fn as_backend(&self) -> &dyn RuntimeBackend {
        match self {
            Self::Docker(b) => b,
            Self::LocalProcess(b) => b,
        }
    }
}
