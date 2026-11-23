//! The Nexus server.
//!
//! The server is authoritative for workspace state, sessions, permissions,
//! Cubes, events, and collaboration state. Clients maintain synchronized local
//! representations but never become independent sources of truth.
//!
//! Part 3 layers a real network server on the Part 1/2 foundation:
//!
//! - protocol handshake (version + capability negotiation),
//! - trust-based authentication with per-peer rate limiting,
//! - authoritative server-side sessions,
//! - per-Cube workspaces over a pluggable storage backends,
//! - a permission matrix (Owner/Editor/Viewer) with a membership gate,
//! - cube-scoped event routing to live connections,
//! - graceful, signal-driven shutdown.

pub mod auth;
pub mod build;
pub mod collaboration;
pub mod connection;
pub mod diagnostics;
pub mod events;
pub mod health;
pub mod lifecycle;
pub mod manager;
pub mod metrics;
pub mod permissions;
pub mod plugins;
pub mod protocol;
pub mod recovery;
pub mod server;
pub mod session;
pub mod shutdown;
pub mod storage;
pub mod terminals;
pub mod users;
pub mod workspace;

pub use collaboration::{CollabEphemeralEvent, CollaborationManager, CubeCollaborationState};
pub use connection::ConnectionManager;
pub use diagnostics::{collect, install_crash_handler, CrashReport, DiagnosticReport};
pub use events::EventRouter;
pub use health::{HealthComponent, HealthStatus};
pub use lifecycle::{LifecycleHandle, ServerLifecycle};
pub use manager::InMemoryCubeManager;
pub use metrics::Metrics;
pub use recovery::{RecoveryConfig, RecoveryManager};
pub use server::{RunningServer, Server, ServerState};
pub use shutdown::{spawn_signal_task, Shutdown};
pub use storage::{FilesystemStorage, InMemoryStorage, Storage};

/// The name reported by this server during the protocol handshake.
pub const SERVER_NAME: &str = "nexus-server";
