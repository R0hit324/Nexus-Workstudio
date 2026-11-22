//! `nexus-core` contains the fundamental domain types shared by every Nexus
//! crate: strongly typed identifiers, timestamps, versions, permissions,
//! errors, configuration, logging, and the foundational abstractions for
//! Cubes, workspaces, sessions, events, and domain services.
//!
//! This crate is intentionally dependency-light and must not depend on
//! networking, storage, or TUI implementation details.

pub mod bench;
pub mod build;
pub mod collaboration;
pub mod config;
pub mod cube;
pub mod error;
pub mod event;
pub mod ids;
pub mod logging;
pub mod metrics;
pub mod permission;
pub mod runtime;
pub mod session;
pub mod terminal;
pub mod timestamp;
pub mod version;
pub mod workspace;

pub use bench::{Benchmark, BenchmarkReport};
pub use build::{
    ArtifactKind, BuildArtifact, BuildConfiguration, BuildPriority, BuildProfile, BuildRequest,
    BuildResult, BuildState, BuildTarget, BuildTargetType, CompilerAdapter, Diagnostic,
    DiagnosticSeverity, EnvVar, LanguageAdapterRegistry, ResourceUsage, TestCaseResult,
    TestRequest, TestResult,
};
pub use collaboration::{
    ActivityEvent, ActivityEventType, BuildActivity, BuildStatus, ChatMessage, Collaborator,
    CursorPosition, FollowState, PairProgrammingRole, PairSession, PresenceStatus, Selection,
    TerminalActivity, User,
};
pub use config::{ClientConfig, LogFormat, LogLevel, LoggingConfig, NexusConfig, ServerConfig};
pub use error::{Error, ErrorCode, Result};
pub use event::{DeliveryClass, EventCategory, EventSequence};
pub use ids::{
    ArtifactId, BuildId, ConnectionId, CubeId, EventId, FileId, ProjectId, RequestId, RuntimeId,
    SessionId, SnapshotId, TerminalId, UserId, WorkspaceId,
};
pub use metrics::{PerformanceMetrics, PerformanceSnapshot};
pub use permission::Permission;
pub use runtime::{
    AutoStartPolicy, ExecRequest, ExecResult, HealthCheckConfig, HealthStatus, MountConfig,
    MountType, NetworkConfig, PortMapping, PortProtocol, ProcessInfo, ProcessStatus,
    ResourceLimits, RuntimeBackend, RuntimeCommands, RuntimeDiagnostics, RuntimeImage,
    RuntimeManifest, RuntimeMetadata, RuntimeProfile, RuntimeStatus,
};
pub use terminal::{
    InputOwnership, TerminalControlRequest, TerminalEvent, TerminalPermission, TerminalSession,
    TerminalSize, TerminalState, TerminalVisibility,
};
pub use timestamp::Timestamp;
pub use version::Version;
