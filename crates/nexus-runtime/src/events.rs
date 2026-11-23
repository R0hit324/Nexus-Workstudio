//! Runtime events.

use nexus_core::{CubeId, RuntimeId, RuntimeStatus, Timestamp};
use serde::{Deserialize, Serialize};

/// Runtime lifecycle events.
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
    StatusChanged {
        runtime_id: RuntimeId,
        status: RuntimeStatus,
    },
}

/// Workspace change events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceChangeEvent {
    pub cube_id: CubeId,
    pub file_id: nexus_core::FileId,
    pub path: std::path::PathBuf,
    pub kind: FileChangeKind,
    pub timestamp: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileChangeKind {
    Created,
    Modified,
    Deleted,
    Renamed,
}

/// Process events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProcessEvent {
    Started {
        process_id: u32,
        runtime_id: RuntimeId,
        cube_id: CubeId,
        command: String,
        timestamp: Timestamp,
    },
    Exited {
        process_id: u32,
        runtime_id: RuntimeId,
        exit_code: i32,
        timestamp: Timestamp,
    },
    Failed {
        process_id: u32,
        runtime_id: RuntimeId,
        error: String,
        timestamp: Timestamp,
    },
}

/// Health events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthEvent {
    pub runtime_id: RuntimeId,
    pub health: nexus_core::HealthStatus,
    pub timestamp: Timestamp,
    pub issues: Vec<String>,
}
