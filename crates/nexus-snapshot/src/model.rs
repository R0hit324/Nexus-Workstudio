use nexus_core::ids::{FileId, SnapshotId};
use nexus_core::timestamp::Timestamp;
use nexus_core::version::Version;
use serde::{Deserialize, Serialize};

use crate::metadata::SnapshotMetadata;

/// Type of snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotType {
    /// Full snapshot containing complete workspace state.
    Full,
    /// Incremental snapshot containing changes since base snapshot.
    Incremental { base_snapshot_id: SnapshotId },
}

/// Status of a snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotStatus {
    /// Snapshot is being created.
    Creating,
    /// Snapshot is complete and valid.
    Active,
    /// Snapshot is corrupted or invalid.
    Corrupted,
    /// Snapshot is marked for deletion.
    Deleted,
    /// Snapshot is protected from retention policies.
    Protected,
}

/// Optional label for a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotLabel {
    pub key: String,
    pub value: String,
}

impl SnapshotLabel {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }

    /// Creates a label for marking a snapshot as protected.
    pub fn protected() -> Self {
        Self::new("protected", "true")
    }

    /// Creates a label with a custom description.
    pub fn description(desc: impl Into<String>) -> Self {
        Self::new("description", desc.into())
    }

    /// Creates a label indicating the reason for snapshot creation.
    pub fn reason(reason: impl Into<String>) -> Self {
        Self::new("reason", reason.into())
    }
}

/// State contained within a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotState {
    /// Cube metadata at snapshot time.
    pub cube_metadata: nexus_core::cube::CubeDescriptor,
    /// Workspace metadata.
    pub workspace_metadata: nexus_core::workspace::WorkspaceMetadata,
    /// Directory structure.
    pub directories: Vec<nexus_core::workspace::DirectoryEntry>,
    /// File metadata (without contents for incremental).
    pub file_metadata: Vec<nexus_core::workspace::FileEntry>,
    /// File contents (only for full snapshots).
    pub file_contents: Vec<FileContent>,
    /// Cube permissions state.
    pub permissions: PermissionsSnapshot,
    /// Cube membership.
    pub members: Vec<MemberSnapshot>,
}

/// File content for snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileContent {
    pub file_id: FileId,
    pub path: String,
    pub content: Vec<u8>,
    pub content_hash: String,
}

/// Permissions snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionsSnapshot {
    pub rules: Vec<nexus_core::Permission>,
}

/// Member snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberSnapshot {
    pub user_id: nexus_core::ids::UserId,
    pub role: nexus_core::Permission,
    pub joined_at: Timestamp,
}

/// Complete workspace snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSnapshot {
    pub snapshot_id: SnapshotId,
    pub cube_id: nexus_core::ids::CubeId,
    pub snapshot_type: SnapshotType,
    pub status: SnapshotStatus,
    pub event_sequence: nexus_core::event::EventSequence,
    pub workspace_version: Version,
    pub created_at: Timestamp,
    pub schema_version: u32,
    pub state: SnapshotState,
    pub labels: Vec<SnapshotLabel>,
    pub metadata: SnapshotMetadata,
    /// Compressed size in bytes.
    pub compressed_size: u64,
    /// Uncompressed size in bytes.
    pub uncompressed_size: u64,
    /// Checksum of the compressed data.
    pub checksum: String,
}

impl WorkspaceSnapshot {
    /// Current schema version for snapshots.
    pub const CURRENT_SCHEMA_VERSION: u32 = 1;

    /// Creates a new full snapshot.
    pub fn new_full(
        cube_id: nexus_core::ids::CubeId,
        event_sequence: nexus_core::event::EventSequence,
        workspace_version: Version,
        state: SnapshotState,
    ) -> Self {
        Self {
            snapshot_id: SnapshotId::new(),
            cube_id,
            snapshot_type: SnapshotType::Full,
            status: SnapshotStatus::Creating,
            event_sequence,
            workspace_version,
            created_at: Timestamp::now(),
            schema_version: Self::CURRENT_SCHEMA_VERSION,
            state,
            labels: Vec::new(),
            metadata: SnapshotMetadata::default(),
            compressed_size: 0,
            uncompressed_size: 0,
            checksum: String::new(),
        }
    }

    /// Creates a new incremental snapshot.
    pub fn new_incremental(
        cube_id: nexus_core::ids::CubeId,
        base_snapshot_id: SnapshotId,
        event_sequence: nexus_core::event::EventSequence,
        workspace_version: Version,
        state: SnapshotState,
    ) -> Self {
        Self {
            snapshot_id: SnapshotId::new(),
            cube_id,
            snapshot_type: SnapshotType::Incremental { base_snapshot_id },
            status: SnapshotStatus::Creating,
            event_sequence,
            workspace_version,
            created_at: Timestamp::now(),
            schema_version: Self::CURRENT_SCHEMA_VERSION,
            state,
            labels: Vec::new(),
            metadata: SnapshotMetadata::default(),
            compressed_size: 0,
            uncompressed_size: 0,
            checksum: String::new(),
        }
    }

    /// Marks the snapshot as active.
    pub fn mark_active(&mut self) {
        self.status = SnapshotStatus::Active;
    }

    /// Marks the snapshot as corrupted.
    pub fn mark_corrupted(&mut self) {
        self.status = SnapshotStatus::Corrupted;
    }

    /// Marks the snapshot as protected.
    pub fn protect(&mut self) {
        self.status = SnapshotStatus::Protected;
        self.labels.push(SnapshotLabel::protected());
    }

    /// Checks if the snapshot is protected.
    pub fn is_protected(&self) -> bool {
        self.status == SnapshotStatus::Protected
            || self
                .labels
                .iter()
                .any(|l| l.key == "protected" && l.value == "true")
    }

    /// Adds a label.
    pub fn add_label(&mut self, label: SnapshotLabel) {
        self.labels.push(label);
    }

    /// Checks if the snapshot is a full snapshot.
    pub fn is_full(&self) -> bool {
        matches!(self.snapshot_type, SnapshotType::Full)
    }

    /// Checks if the snapshot is an incremental snapshot.
    pub fn is_incremental(&self) -> bool {
        matches!(self.snapshot_type, SnapshotType::Incremental { .. })
    }

    /// Gets the base snapshot ID for incremental snapshots.
    pub fn base_snapshot_id(&self) -> Option<SnapshotId> {
        match self.snapshot_type {
            SnapshotType::Incremental { base_snapshot_id } => Some(base_snapshot_id),
            SnapshotType::Full => None,
        }
    }

    /// Validates the snapshot against its schema version.
    pub fn validate_schema(&self) -> Result<(), SnapshotValidationError> {
        if self.schema_version > Self::CURRENT_SCHEMA_VERSION {
            return Err(SnapshotValidationError::FutureSchemaVersion {
                found: self.schema_version,
                current: Self::CURRENT_SCHEMA_VERSION,
            });
        }
        Ok(())
    }
}

/// Errors related to snapshot validation.
#[derive(Debug, thiserror::Error)]
pub enum SnapshotValidationError {
    #[error("future schema version: found {found}, current {current}")]
    FutureSchemaVersion { found: u32, current: u32 },

    #[error("invalid schema version: {0}")]
    InvalidSchemaVersion(u32),

    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },

    #[error("corrupted data: {0}")]
    CorruptedData(String),

    #[error("invalid cube ID: expected {expected}, got {actual}")]
    CubeIdMismatch {
        expected: nexus_core::ids::CubeId,
        actual: nexus_core::ids::CubeId,
    },

    #[error("invalid snapshot type for operation: {0}")]
    InvalidSnapshotType(String),

    #[error("missing base snapshot for incremental: {0}")]
    MissingBaseSnapshot(SnapshotId),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    SerializationError(#[from] bincode::Error),

    #[error("compression error: {0}")]
    CompressionError(#[from] crate::compression::CompressionError),
}
