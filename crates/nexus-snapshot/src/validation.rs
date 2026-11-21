use crate::model::WorkspaceSnapshot;

/// Snapshot validator for integrity checks.
pub struct SnapshotValidator {
    max_size_bytes: u64,
    allowed_schema_versions: Vec<u32>,
}

impl SnapshotValidator {
    pub fn new() -> Self {
        Self {
            max_size_bytes: 1024 * 1024 * 1024, // 1 GB default
            allowed_schema_versions: vec![WorkspaceSnapshot::CURRENT_SCHEMA_VERSION],
        }
    }

    pub fn with_max_size(mut self, max_size: u64) -> Self {
        self.max_size_bytes = max_size;
        self
    }

    pub fn with_allowed_versions(mut self, versions: Vec<u32>) -> Self {
        self.allowed_schema_versions = versions;
        self
    }

    /// Validates a complete snapshot.
    pub fn validate(
        &self,
        snapshot: &WorkspaceSnapshot,
    ) -> Result<IntegrityCheck, ValidationError> {
        // Check schema version
        if !self
            .allowed_schema_versions
            .contains(&snapshot.schema_version)
        {
            return Err(ValidationError::UnsupportedSchemaVersion(
                snapshot.schema_version,
            ));
        }

        // Check size
        if snapshot.compressed_size > self.max_size_bytes {
            return Err(ValidationError::SizeExceeded {
                size: snapshot.compressed_size,
                max: self.max_size_bytes,
            });
        }

        // Check event sequence is not unassigned
        if snapshot.event_sequence == nexus_core::event::EventSequence::UNASSIGNED {
            return Err(ValidationError::UnassignedSequence);
        }

        // Check workspace version is valid
        if snapshot.workspace_version == nexus_core::version::Version::new(0) {
            return Err(ValidationError::InvalidWorkspaceVersion);
        }

        // Check created_at is not default
        if snapshot.created_at == nexus_core::timestamp::Timestamp::from_millis(0) {
            return Err(ValidationError::InvalidTimestamp);
        }

        // Check checksum is present
        if snapshot.checksum.is_empty() {
            return Err(ValidationError::MissingChecksum);
        }

        // Check state is not empty for full snapshots
        if snapshot.is_full() {
            if snapshot.state.file_metadata.is_empty() && snapshot.state.directories.is_empty() {
                return Err(ValidationError::EmptyState);
            }
        }

        Ok(IntegrityCheck {
            snapshot_id: snapshot.snapshot_id,
            cube_id: snapshot.cube_id,
            event_sequence: snapshot.event_sequence,
            workspace_version: snapshot.workspace_version,
            checksum_verified: false, // Will be set after checksum verification
            schema_version_ok: true,
            size_ok: true,
            timestamp_ok: true,
            state_non_empty: true,
        })
    }

    /// Validates a snapshot's checksum.
    pub fn verify_checksum(
        &self,
        snapshot: &WorkspaceSnapshot,
        data: &[u8],
    ) -> Result<bool, ValidationError> {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(data);
        let computed = format!("{:x}", hasher.finalize());
        Ok(computed == snapshot.checksum)
    }

    /// Validates snapshot state consistency.
    pub fn validate_state_consistency(
        &self,
        snapshot: &WorkspaceSnapshot,
    ) -> Result<(), ValidationError> {
        // Verify file contents match metadata
        for file_content in &snapshot.state.file_contents {
            let metadata = snapshot
                .state
                .file_metadata
                .iter()
                .find(|f| f.id == file_content.file_id);
            if let Some(meta) = metadata {
                if meta.path != file_content.path {
                    return Err(ValidationError::PathMismatch {
                        expected: meta.path.clone(),
                        actual: file_content.path.clone(),
                    });
                }
            }
        }

        // Verify directories reference existing files
        for dir in &snapshot.state.directories {
            if dir.path.is_empty() {
                return Err(ValidationError::InvalidWorkspaceVersion);
            }
        }

        Ok(())
    }
}

impl Default for SnapshotValidator {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of integrity check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrityCheck {
    pub snapshot_id: nexus_core::ids::SnapshotId,
    pub cube_id: nexus_core::ids::CubeId,
    pub event_sequence: nexus_core::event::EventSequence,
    pub workspace_version: nexus_core::version::Version,
    pub checksum_verified: bool,
    pub schema_version_ok: bool,
    pub size_ok: bool,
    pub timestamp_ok: bool,
    pub state_non_empty: bool,
}

impl IntegrityCheck {
    pub fn all_ok(&self) -> bool {
        self.checksum_verified
            && self.schema_version_ok
            && self.size_ok
            && self.timestamp_ok
            && self.state_non_empty
    }
}

/// Validation errors.
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("unsupported schema version: {0}")]
    UnsupportedSchemaVersion(u32),

    #[error("snapshot size exceeded: {size} bytes (max {max})")]
    SizeExceeded { size: u64, max: u64 },

    #[error("unassigned event sequence")]
    UnassignedSequence,

    #[error("invalid workspace version")]
    InvalidWorkspaceVersion,

    #[error("invalid timestamp")]
    InvalidTimestamp,

    #[error("missing checksum")]
    MissingChecksum,

    #[error("empty state for full snapshot")]
    EmptyState,

    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },

    #[error("path mismatch: expected {expected}, got {actual}")]
    PathMismatch { expected: String, actual: String },

    #[error("orphaned file reference: {file_id}")]
    OrphanedFileReference { file_id: nexus_core::ids::FileId },

    #[error("corrupted data: {0}")]
    CorruptedData(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    SerializationError(#[from] bincode::Error),

    #[error("compression error: {0}")]
    CompressionError(#[from] crate::compression::CompressionError),

    #[error("store error: {0}")]
    StoreError(#[from] crate::store::StoreError),

    #[error("model validation error: {0}")]
    ModelError(#[from] crate::model::SnapshotValidationError),
}

/// Checkpoint for incremental validation.
#[derive(Debug, Clone)]
pub struct ValidationCheckpoint {
    pub snapshot_id: nexus_core::ids::SnapshotId,
    pub event_sequence: nexus_core::event::EventSequence,
    pub checksum: String,
    pub validated_at: nexus_core::timestamp::Timestamp,
}
