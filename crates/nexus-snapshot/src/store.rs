use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::metadata::SnapshotMetadata;

/// Snapshot store errors.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("snapshot not found: {0}")]
    NotFound(String),

    #[error("snapshot already exists: {0}")]
    AlreadyExists(String),

    #[error("invalid snapshot path: {0}")]
    InvalidPath(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] bincode::Error),

    #[error("compression error: {0}")]
    Compression(#[from] crate::compression::CompressionError),

    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },

    #[error("concurrent modification")]
    ConcurrentModification,

    #[error("disk full")]
    DiskFull,

    #[error("invalid snapshot state: {0}")]
    InvalidState(String),
}

impl From<uuid::Error> for StoreError {
    fn from(error: uuid::Error) -> Self {
        StoreError::InvalidState(format!("invalid snapshot ID: {}", error))
    }
}

/// Snapshot store trait for abstraction.
#[async_trait::async_trait]
pub trait SnapshotStore: Send + Sync {
    /// Writes a snapshot atomically.
    async fn write(&self, snapshot: &crate::model::WorkspaceSnapshot) -> Result<(), StoreError>;

    /// Reads a snapshot by ID.
    async fn read(
        &self,
        snapshot_id: nexus_core::ids::SnapshotId,
    ) -> Result<crate::model::WorkspaceSnapshot, StoreError>;

    /// Deletes a snapshot.
    async fn delete(&self, snapshot_id: nexus_core::ids::SnapshotId) -> Result<(), StoreError>;

    /// Lists all snapshots for a cube.
    async fn list(
        &self,
        cube_id: nexus_core::ids::CubeId,
    ) -> Result<Vec<SnapshotMetadata>, StoreError>;

    /// Gets the latest active snapshot for a cube.
    async fn get_latest(
        &self,
        cube_id: nexus_core::ids::CubeId,
    ) -> Result<Option<SnapshotMetadata>, StoreError>;

    /// Gets metadata for a specific snapshot.
    async fn get_metadata(
        &self,
        snapshot_id: nexus_core::ids::SnapshotId,
    ) -> Result<Option<SnapshotMetadata>, StoreError>;

    /// Finds the best snapshot for recovery at a given sequence.
    async fn find_best_for_recovery(
        &self,
        cube_id: nexus_core::ids::CubeId,
        target_sequence: nexus_core::event::EventSequence,
    ) -> Result<Option<SnapshotMetadata>, StoreError>;
}

/// Filesystem-based snapshot store.
#[derive(Debug)]
pub struct FileSnapshotStore {
    root: std::path::PathBuf,
    index: Arc<crate::metadata::SnapshotIndex>,
    temp_dir: std::path::PathBuf,
}

impl FileSnapshotStore {
    /// Creates a new file snapshot store.
    pub fn new(root: impl AsRef<std::path::Path>) -> Result<Self, StoreError> {
        let root = root.as_ref().to_path_buf();
        let temp_dir = root.join(".tmp");

        // Create directories
        fs::create_dir_all(&root)?;
        fs::create_dir_all(&temp_dir)?;

        Ok(Self {
            root,
            index: Arc::new(crate::metadata::SnapshotIndex::new()),
            temp_dir,
        })
    }

    /// Opens an existing snapshot store.
    pub fn open(root: impl AsRef<std::path::Path>) -> Result<Self, StoreError> {
        let root = root.as_ref().to_path_buf();
        let temp_dir = root.join(".tmp");

        let store = Self {
            root,
            index: Arc::new(crate::metadata::SnapshotIndex::new()),
            temp_dir,
        };

        // Load existing snapshots into index
        store.load_index()?;

        Ok(store)
    }

    /// Gets the cube directory path.
    fn cube_dir(&self, cube_id: nexus_core::ids::CubeId) -> std::path::PathBuf {
        self.root
            .join("cubes")
            .join(cube_id.to_string())
            .join("snapshots")
    }

    /// Gets the snapshot file path.
    fn snapshot_path(
        &self,
        cube_id: nexus_core::ids::CubeId,
        snapshot_id: nexus_core::ids::SnapshotId,
    ) -> std::path::PathBuf {
        self.cube_dir(cube_id).join(format!(
            "snapshot-{:06}.bin",
            snapshot_id.as_uuid().as_u128() & 0xFFFFFF
        ))
    }

    /// Gets the metadata file path.
    fn metadata_path(
        &self,
        cube_id: nexus_core::ids::CubeId,
        snapshot_id: nexus_core::ids::SnapshotId,
    ) -> std::path::PathBuf {
        self.cube_dir(cube_id).join(format!(
            "snapshot-{:06}.meta",
            snapshot_id.as_uuid().as_u128() & 0xFFFFFF
        ))
    }

    /// Gets the temporary file path.
    fn temp_path(&self, snapshot_id: nexus_core::ids::SnapshotId) -> std::path::PathBuf {
        self.temp_dir.join(format!("tmp-{}.tmp", snapshot_id))
    }

    /// Loads the index from existing snapshots.
    fn load_index(&self) -> Result<(), StoreError> {
        let cubes_dir = self.root.join("cubes");
        if !cubes_dir.exists() {
            return Ok(());
        }

        for cube_entry in fs::read_dir(cubes_dir)? {
            let cube_entry = cube_entry?;
            let cube_id_str = cube_entry.file_name().to_string_lossy().to_string();
            let _cube_id = nexus_core::ids::CubeId::from(uuid::Uuid::parse_str(&cube_id_str)?);

            let snapshots_dir = cube_entry.path().join("snapshots");
            if !snapshots_dir.exists() {
                continue;
            }

            for entry in fs::read_dir(snapshots_dir)? {
                let entry = entry?;
                if entry
                    .path()
                    .extension()
                    .map(|e| e == "meta")
                    .unwrap_or(false)
                {
                    if let Ok(metadata) = self.read_metadata_file(&entry.path()) {
                        self.index.insert(metadata);
                    }
                }
            }
        }

        Ok(())
    }

    /// Reads metadata from a metadata file.
    fn read_metadata_file(&self, path: &std::path::Path) -> Result<SnapshotMetadata, StoreError> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let metadata: SnapshotMetadata = bincode::deserialize_from(reader)?;
        Ok(metadata)
    }

    /// Writes metadata to a metadata file.
    fn write_metadata_file(
        &self,
        path: &std::path::Path,
        metadata: &SnapshotMetadata,
    ) -> Result<(), StoreError> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);
        bincode::serialize_into(&mut writer, metadata)?;
        writer.flush()?;
        Ok(())
    }

    /// Computes SHA-256 checksum.
    fn compute_checksum(&self, data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        format!("{:x}", hasher.finalize())
    }
}

#[async_trait::async_trait]
impl crate::store::SnapshotStore for FileSnapshotStore {
    async fn write(&self, snapshot: &crate::model::WorkspaceSnapshot) -> Result<(), StoreError> {
        let cube_id = snapshot.cube_id;
        let snapshot_id = snapshot.snapshot_id;

        // Ensure cube directory exists
        let cube_dir = self.cube_dir(cube_id);
        fs::create_dir_all(&cube_dir)?;

        // Serialize snapshot
        let data = bincode::serialize(snapshot)?;

        // Compress if needed
        let compressor = crate::compression::Compressor::new();
        let (compressed_data, _) = compressor.compress_with_ratio(&data)?;

        // Compute checksum
        let checksum = self.compute_checksum(&compressed_data);

        // Write to temporary file
        let temp_path = self.temp_path(snapshot_id);
        {
            let mut file = File::create(&temp_path)?;
            file.write_all(&compressed_data)?;
            file.flush()?;
        }

        // Validate by reading back
        let mut verify_data = Vec::new();
        {
            let mut file = File::open(&temp_path)?;
            file.read_to_end(&mut verify_data)?;
        }
        if self.compute_checksum(&verify_data) != checksum {
            return Err(StoreError::InvalidState(format!(
                "Checksum mismatch after write: expected {}, got {}",
                checksum,
                self.compute_checksum(&verify_data)
            )));
        }

        // Atomically move to final location
        let snapshot_path = self.snapshot_path(cube_id, snapshot_id);
        fs::rename(&temp_path, &snapshot_path)?;

        // Create metadata from the snapshot
        let metadata = SnapshotMetadata {
            snapshot_id: snapshot.snapshot_id,
            cube_id: snapshot.cube_id,
            event_sequence: snapshot.event_sequence,
            workspace_version: snapshot.workspace_version,
            created_at: snapshot.created_at,
            size_bytes: compressed_data.len() as u64,
            checksum: checksum.clone(),
            schema_version: snapshot.schema_version,
            snapshot_type: snapshot.snapshot_type,
            status: crate::model::SnapshotStatus::Active,
            labels: snapshot.labels.clone(),
            compressed_size: compressed_data.len() as u64,
            uncompressed_size: data.len() as u64,
        };

        let metadata_path = self.metadata_path(cube_id, snapshot_id);
        self.write_metadata_file(&metadata_path, &metadata)?;

        // Update index
        self.index.insert(metadata);

        Ok(())
    }

    async fn read(
        &self,
        snapshot_id: nexus_core::ids::SnapshotId,
    ) -> Result<crate::model::WorkspaceSnapshot, StoreError> {
        // Find the cube_id by searching metadata
        let metadata = self
            .index
            .get(snapshot_id)
            .ok_or_else(|| StoreError::NotFound(snapshot_id.to_string()))?;

        let snapshot_path = self.snapshot_path(metadata.cube_id, snapshot_id);
        let mut file = File::open(&snapshot_path)?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;

        // Decompress
        let compressor = crate::compression::Compressor::new();
        let decompressed = compressor.decompress(&data)?;

        // Deserialize
        let snapshot: crate::model::WorkspaceSnapshot = bincode::deserialize(&decompressed)?;

        // Verify checksum
        let expected_checksum = metadata.checksum.clone();
        let actual_checksum = self.compute_checksum(&data);
        if expected_checksum != actual_checksum {
            return Err(StoreError::ChecksumMismatch {
                expected: expected_checksum,
                actual: actual_checksum,
            });
        }

        Ok(snapshot)
    }

    async fn delete(&self, snapshot_id: nexus_core::ids::SnapshotId) -> Result<(), StoreError> {
        let metadata = self
            .index
            .get(snapshot_id)
            .ok_or_else(|| StoreError::NotFound(snapshot_id.to_string()))?;

        let snapshot_path = self.snapshot_path(metadata.cube_id, snapshot_id);
        let metadata_path = self.metadata_path(metadata.cube_id, snapshot_id);

        // Delete files
        if snapshot_path.exists() {
            fs::remove_file(snapshot_path)?;
        }
        if metadata_path.exists() {
            fs::remove_file(metadata_path)?;
        }

        // Mark as deleted in index
        self.index.mark_deleted(snapshot_id);

        Ok(())
    }

    async fn list(
        &self,
        cube_id: nexus_core::ids::CubeId,
    ) -> Result<Vec<SnapshotMetadata>, StoreError> {
        Ok(self.index.list_for_cube(cube_id))
    }

    async fn get_latest(
        &self,
        cube_id: nexus_core::ids::CubeId,
    ) -> Result<Option<SnapshotMetadata>, StoreError> {
        let snapshot_id = self
            .index
            .get_latest(cube_id)
            .ok_or(StoreError::NotFound(format!(
                "no latest snapshot for cube {:?}",
                cube_id
            )))?;
        let metadata = self.index.get(snapshot_id);
        Ok(metadata)
    }

    async fn get_metadata(
        &self,
        snapshot_id: nexus_core::ids::SnapshotId,
    ) -> Result<Option<SnapshotMetadata>, StoreError> {
        Ok(self.index.get(snapshot_id))
    }

    async fn find_best_for_recovery(
        &self,
        cube_id: nexus_core::ids::CubeId,
        target_sequence: nexus_core::event::EventSequence,
    ) -> Result<Option<SnapshotMetadata>, StoreError> {
        Ok(self.index.find_best_for_recovery(cube_id, target_sequence))
    }
}

impl FileSnapshotStore {
    /// Gets the index for direct access.
    pub fn index(&self) -> &Arc<crate::metadata::SnapshotIndex> {
        &self.index
    }
}
