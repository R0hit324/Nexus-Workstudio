use serde::{Deserialize, Serialize};

use crate::ids::{CubeId, RuntimeId, UserId};
use crate::timestamp::Timestamp;
use crate::version::Version;

/// Runtime lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RuntimeStatus {
    Created,
    Preparing,
    Starting,
    Running,
    Stopping,
    Stopped,
    Restarting,
    Failed,
    Destroyed,
}

impl RuntimeStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Stopped | Self::Failed | Self::Destroyed)
    }

    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running)
    }

    pub fn can_transition_to(&self, target: Self) -> bool {
        use RuntimeStatus::*;
        match (self, target) {
            (Created, Preparing) => true,
            (Preparing, Starting) => true,
            (Starting, Running) => true,
            (Starting, Failed) => true,
            (Running, Stopping) => true,
            (Running, Restarting) => true,
            (Stopping, Stopped) => true,
            (Stopped, Starting) => true,
            (Stopped, Destroyed) => true,
            (Restarting, Starting) => true,
            (Failed, Restarting) => true,
            (Failed, Destroyed) => true,
            _ => false,
        }
    }
}

/// Runtime backend type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RuntimeBackend {
    Docker,
    LocalProcess,
    Podman,
}

/// Resource limits for a runtime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// CPU cores (e.g., 2.0 = 2 cores)
    pub cpu_cores: Option<f64>,
    /// Memory limit in bytes
    pub memory_bytes: Option<u64>,
    /// Disk limit in bytes
    pub disk_bytes: Option<u64>,
    /// Maximum process count
    pub max_processes: Option<u32>,
    /// Network bandwidth limit (bytes/sec)
    pub network_bandwidth: Option<u64>,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            cpu_cores: Some(2.0),
            memory_bytes: Some(2 * 1024 * 1024 * 1024), // 2GB
            disk_bytes: Some(10 * 1024 * 1024 * 1024),  // 10GB
            max_processes: Some(100),
            network_bandwidth: None,
        }
    }
}

/// Network configuration for a runtime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// Enable network access
    pub enabled: bool,
    /// Allowed outbound destinations (CIDR)
    pub allowed_cidrs: Vec<String>,
    /// DNS servers
    pub dns_servers: Vec<String>,
    /// Host network mode
    pub host_network: bool,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            allowed_cidrs: vec!["192.168.0.0/16".into(), "10.0.0.0/8".into()],
            dns_servers: vec!["8.8.8.8".into(), "1.1.1.1".into()],
            host_network: false,
        }
    }
}

/// Port mapping configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortMapping {
    /// Container port
    pub container_port: u16,
    /// Host port (0 = auto-allocate)
    pub host_port: u16,
    /// Protocol (tcp/udp)
    pub protocol: PortProtocol,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PortProtocol {
    Tcp,
    Udp,
}

/// Environment variable configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvVar {
    pub key: String,
    pub value: String,
    pub is_secret: bool,
}

/// Runtime image configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeImage {
    /// Image name (e.g., rust:stable)
    pub image: String,
    /// Architecture (amd64, arm64)
    pub arch: String,
    /// Platform variant
    pub variant: Option<String>,
}

/// Runtime profile for language-specific defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RuntimeProfile {
    Rust,
    Python,
    Node,
    Java,
    Go,
    Cpp,
    Generic,
}

impl RuntimeProfile {
    pub fn default_image(&self) -> &'static str {
        match self {
            Self::Rust => "rust:stable",
            Self::Python => "python:3.12",
            Self::Node => "node:22",
            Self::Java => "eclipse-temurin:21",
            Self::Go => "golang:1.22",
            Self::Cpp => "gcc:13",
            Self::Generic => "ubuntu:22.04",
        }
    }

    pub fn default_build_cmd(&self) -> Option<&'static str> {
        match self {
            Self::Rust => Some("cargo build"),
            Self::Python => None,
            Self::Node => Some("npm run build"),
            Self::Java => Some("./mvnw package"),
            Self::Go => Some("go build"),
            Self::Cpp => Some("cmake --build ."),
            Self::Generic => None,
        }
    }

    pub fn default_run_cmd(&self) -> Option<&'static str> {
        match self {
            Self::Rust => Some("cargo run"),
            Self::Python => Some("python main.py"),
            Self::Node => Some("npm start"),
            Self::Java => Some("java -jar app.jar"),
            Self::Go => Some("go run ."),
            Self::Cpp => Some("./app"),
            Self::Generic => None,
        }
    }

    pub fn default_test_cmd(&self) -> Option<&'static str> {
        match self {
            Self::Rust => Some("cargo test"),
            Self::Python => Some("pytest"),
            Self::Node => Some("npm test"),
            Self::Java => Some("./mvnw test"),
            Self::Go => Some("go test ./..."),
            Self::Cpp => Some("ctest"),
            Self::Generic => None,
        }
    }
}

/// Auto-start policy for runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutoStartPolicy {
    Never,
    OnCubeJoin,
    OnFirstTerminal,
    OnFirstBuild,
    Always,
}

/// Health check status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

/// Runtime configuration manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeManifest {
    pub version: Version,
    pub backend: RuntimeBackend,
    pub profile: RuntimeProfile,
    pub image: RuntimeImage,
    pub resources: ResourceLimits,
    pub network: NetworkConfig,
    pub environment: Vec<EnvVar>,
    pub ports: Vec<PortMapping>,
    pub mounts: Vec<MountConfig>,
    pub auto_start: AutoStartPolicy,
    pub idle_timeout: Option<u64>, // seconds
    pub health_check: HealthCheckConfig,
    pub commands: RuntimeCommands,
}

impl Default for RuntimeManifest {
    fn default() -> Self {
        Self {
            version: Version::new(1),
            backend: RuntimeBackend::Docker,
            profile: RuntimeProfile::Generic,
            image: RuntimeImage {
                image: "ubuntu:22.04".into(),
                arch: "amd64".into(),
                variant: None,
            },
            resources: ResourceLimits::default(),
            network: NetworkConfig::default(),
            environment: Vec::new(),
            ports: Vec::new(),
            mounts: Vec::new(),
            auto_start: AutoStartPolicy::Never,
            idle_timeout: None,
            health_check: HealthCheckConfig::default(),
            commands: RuntimeCommands::default(),
        }
    }
}

/// Mount configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MountConfig {
    pub source: String,
    pub target: String,
    pub read_only: bool,
    pub mount_type: MountType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MountType {
    Bind,
    Volume,
    Tmpfs,
}

/// Health check configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheckConfig {
    pub enabled: bool,
    pub interval_seconds: u64,
    pub timeout_seconds: u64,
    pub retries: u32,
    pub command: Option<String>,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_seconds: 30,
            timeout_seconds: 10,
            retries: 3,
            command: None,
        }
    }
}

/// Runtime commands configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeCommands {
    pub build: Option<String>,
    pub run: Option<String>,
    pub test: Option<String>,
    pub debug: Option<String>,
    pub install_deps: Option<String>,
}

impl Default for RuntimeCommands {
    fn default() -> Self {
        Self {
            build: None,
            run: None,
            test: None,
            debug: None,
            install_deps: None,
        }
    }
}

/// Runtime metadata for persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeMetadata {
    pub runtime_id: RuntimeId,
    pub cube_id: CubeId,
    pub backend: RuntimeBackend,
    pub manifest: RuntimeManifest,
    pub status: RuntimeStatus,
    pub created_at: Timestamp,
    pub started_at: Option<Timestamp>,
    pub stopped_at: Option<Timestamp>,
    pub assigned_ports: Vec<PortMapping>,
    pub container_id: Option<String>,
}

/// Process information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub process_id: u32,
    pub runtime_id: RuntimeId,
    pub cube_id: CubeId,
    pub user_id: UserId,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<EnvVar>,
    pub working_dir: String,
    pub started_at: Timestamp,
    pub status: ProcessStatus,
    pub exit_code: Option<i32>,
    pub cpu_usage: f64,
    pub memory_bytes: u64,
}

/// Process lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProcessStatus {
    Starting,
    Running,
    Stopping,
    Exited,
    Failed,
    Killed,
}

/// Exec request for running commands in runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecRequest {
    pub runtime_id: RuntimeId,
    pub cube_id: CubeId,
    pub user_id: UserId,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<EnvVar>,
    pub working_dir: Option<String>,
    pub tty: bool,
    pub shared: bool,
}

/// Exec result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResult {
    pub process_id: u32,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Runtime diagnostic information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeDiagnostics {
    pub runtime_id: RuntimeId,
    pub status: RuntimeStatus,
    pub health: HealthStatus,
    pub cpu_usage: f64,
    pub memory_usage: u64,
    pub disk_usage: u64,
    pub process_count: u32,
    pub uptime_seconds: u64,
    pub issues: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_status_transitions() {
        assert!(RuntimeStatus::Created.can_transition_to(RuntimeStatus::Preparing));
        assert!(RuntimeStatus::Preparing.can_transition_to(RuntimeStatus::Starting));
        assert!(RuntimeStatus::Starting.can_transition_to(RuntimeStatus::Running));
        assert!(RuntimeStatus::Running.can_transition_to(RuntimeStatus::Stopping));
        assert!(RuntimeStatus::Stopping.can_transition_to(RuntimeStatus::Stopped));
        assert!(RuntimeStatus::Stopped.can_transition_to(RuntimeStatus::Starting));
        assert!(!RuntimeStatus::Running.can_transition_to(RuntimeStatus::Created));
    }

    #[test]
    fn runtime_profile_defaults() {
        assert_eq!(RuntimeProfile::Rust.default_image(), "rust:stable");
        assert_eq!(
            RuntimeProfile::Rust.default_build_cmd(),
            Some("cargo build")
        );
        assert_eq!(RuntimeProfile::Python.default_test_cmd(), Some("pytest"));
    }

    #[test]
    fn resource_limits_defaults() {
        let limits = ResourceLimits::default();
        assert_eq!(limits.cpu_cores, Some(2.0));
        assert_eq!(limits.memory_bytes, Some(2 * 1024 * 1024 * 1024));
    }
}
