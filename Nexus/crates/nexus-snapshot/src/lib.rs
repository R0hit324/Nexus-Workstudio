//! Part 6: Workspace Snapshots
//!
//! Atomic snapshot creation, integrity verification, compression, recovery,
//! and retention for Nexus Cube workspaces.

pub mod compression;
pub mod metadata;
pub mod model;
pub mod retention;
pub mod scheduler;
pub mod store;
pub mod validation;

pub use compression::{CompressionAlgorithm, Compressor};
pub use metadata::{SnapshotIndex, SnapshotMetadata};
pub use model::{SnapshotLabel, SnapshotState, SnapshotStatus, SnapshotType, WorkspaceSnapshot};
pub use retention::{RetentionEvent, RetentionPolicy, RetentionResult};
pub use scheduler::{SnapshotScheduler, SnapshotTrigger};
pub use store::{FileSnapshotStore, SnapshotStore};
pub use validation::{IntegrityCheck, SnapshotValidator};

/// Part 6 phase identifier for runtime inspection.
pub const PHASE: &str = "part6-workspace-snapshots";
