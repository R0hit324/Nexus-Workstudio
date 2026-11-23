use std::collections::HashMap;
use std::sync::Arc;

use nexus_core::ids::{CubeId, FileId};
use parking_lot::RwLock;

use crate::{
    cursor::SyncStatus,
    doc::{CollaborationDocument, DocumentState, YrsDocument},
    engine::{SyncEngineFactory, TextSyncEngine},
    error::{SyncError, SyncResult},
    operations::{EditOperation, OperationId, OperationStatus, Version},
};

/// Synchronization state for a single file.
pub struct FileSyncState {
    file_id: FileId,
    document: Arc<dyn CollaborationDocument>,
    version: RwLock<Version>,
    status: RwLock<SyncStatus>,
    last_modified: RwLock<u64>,
    pending_ops: RwLock<HashMap<OperationId, (EditOperation, OperationStatus)>>,
}

impl FileSyncState {
    pub fn new(file_id: FileId, engine: Arc<dyn TextSyncEngine>) -> Self {
        let document = Arc::new(YrsDocument::new(engine, file_id));
        Self {
            file_id,
            document,
            version: RwLock::new(Version::INITIAL),
            status: RwLock::new(SyncStatus::Unknown),
            last_modified: RwLock::new(0),
            pending_ops: RwLock::new(HashMap::new()),
        }
    }

    pub fn with_content(file_id: FileId, engine: Arc<dyn TextSyncEngine>, content: String) -> Self {
        let document = Arc::new(YrsDocument::new(engine, file_id));
        let state = Self {
            file_id,
            document,
            version: RwLock::new(Version::INITIAL),
            status: RwLock::new(SyncStatus::Unknown),
            last_modified: RwLock::new(0),
            pending_ops: RwLock::new(HashMap::new()),
        };
        // Initialize with content
        if !content.is_empty() {
            state.resync(content, Version::INITIAL).ok();
        }
        state
    }

    pub fn apply_local(&self, operation: EditOperation) -> SyncResult<()> {
        self.document.apply_local(operation)?;
        *self.version.write() = self.document.version();
        Ok(())
    }

    pub fn apply_remote(&self, operation: EditOperation) -> SyncResult<()> {
        self.document.apply_remote(operation)?;
        *self.version.write() = self.document.version();
        Ok(())
    }

    pub fn acknowledge(&self, operation_id: OperationId, version: Version) -> SyncResult<()> {
        self.document.acknowledge(operation_id, version)?;
        *self.version.write() = version;
        self.pending_ops
            .write()
            .retain(|_, (_, status)| *status != OperationStatus::Acknowledged);
        Ok(())
    }

    pub fn get_state(&self) -> DocumentState {
        DocumentState {
            file_id: self.file_id,
            version: *self.version.read(),
            status: *self.status.read(),
            content: self.document.content(),
            pending_operations: self.document.pending_operations(),
            last_synced: Some(*self.last_modified.read()),
        }
    }

    pub fn resync(&self, content: String, version: Version) -> SyncResult<()> {
        self.document.resynchronize(content, version)?;
        *self.version.write() = version;
        *self.status.write() = SyncStatus::Synchronized;
        Ok(())
    }

    pub fn request_delta(&self, from_version: Version) -> SyncResult<Vec<EditOperation>> {
        self.document.request_delta(from_version)
    }

    pub fn document(&self) -> &Arc<dyn CollaborationDocument> {
        &self.document
    }

    pub fn content(&self) -> String {
        self.document.content()
    }

    pub fn version(&self) -> Version {
        *self.version.read()
    }

    pub fn file_id(&self) -> FileId {
        self.file_id
    }
}

/// Manages synchronization state for all files in a Cube.
pub struct FileSyncManager {
    #[allow(dead_code)]
    cube_id: CubeId,
    files: RwLock<HashMap<FileId, Arc<FileSyncState>>>,
    engine_factory: Arc<dyn Fn() -> Arc<dyn TextSyncEngine> + Send + Sync>,
}

impl FileSyncManager {
    pub fn new(cube_id: CubeId) -> Self {
        Self {
            cube_id,
            files: RwLock::new(HashMap::new()),
            engine_factory: Arc::new(|| SyncEngineFactory::create_yrs()),
        }
    }

    pub fn with_engine_factory<F>(cube_id: CubeId, factory: F) -> Self
    where
        F: Fn() -> Arc<dyn TextSyncEngine> + Send + Sync + 'static,
    {
        Self {
            cube_id,
            files: RwLock::new(HashMap::new()),
            engine_factory: Arc::new(factory),
        }
    }

    /// Create or get a file's sync state.
    pub fn get_or_create(&self, file_id: FileId) -> Arc<FileSyncState> {
        let mut files = self.files.write();
        files
            .entry(file_id)
            .or_insert_with(|| Arc::new(FileSyncState::new(file_id, (self.engine_factory)())))
            .clone()
    }

    /// Create a file with initial content.
    pub fn create_with_content(&self, file_id: FileId, content: String) -> Arc<FileSyncState> {
        let mut files = self.files.write();
        let state = Arc::new(FileSyncState::with_content(
            file_id,
            (self.engine_factory)(),
            content,
        ));
        files.insert(file_id, state.clone());
        state.clone()
    }

    /// Get existing file state.
    pub fn get(&self, file_id: FileId) -> Option<Arc<FileSyncState>> {
        self.files.read().get(&file_id).cloned()
    }

    /// Remove a file (deletion).
    pub fn remove(&self, file_id: FileId) -> Option<Arc<FileSyncState>> {
        self.files.write().remove(&file_id)
    }

    /// Rename a file (update file_id mapping).
    pub fn rename(&self, old_id: FileId, new_id: FileId) -> Option<Arc<FileSyncState>> {
        let mut files = self.files.write();
        if let Some(state) = files.remove(&old_id) {
            // Create new state with same content but new file_id
            let new_state = Arc::new(FileSyncState::with_content(
                new_id,
                (self.engine_factory)(),
                state.content(),
            ));
            files.insert(new_id, new_state.clone());
            Some(new_state)
        } else {
            None
        }
    }

    /// List all file IDs.
    pub fn list_files(&self) -> Vec<FileId> {
        self.files.read().keys().copied().collect()
    }

    /// Get all file states.
    pub fn all_states(&self) -> Vec<Arc<FileSyncState>> {
        self.files.read().values().cloned().collect()
    }

    /// Apply local edit to a file.
    pub fn apply_local_edit(&self, file_id: FileId, operation: EditOperation) -> SyncResult<()> {
        let state = self.get_or_create(file_id);
        state.apply_local(operation)
    }

    /// Apply remote edit to a file.
    pub fn apply_remote_edit(&self, file_id: FileId, operation: EditOperation) -> SyncResult<()> {
        let state = self.get_or_create(file_id);
        state.apply_remote(operation)
    }

    /// Acknowledge operation.
    pub fn acknowledge_operation(
        &self,
        file_id: FileId,
        operation_id: OperationId,
        version: Version,
    ) -> SyncResult<()> {
        if let Some(state) = self.get(file_id) {
            state.acknowledge(operation_id, version)
        } else {
            Err(SyncError::FileNotFound(file_id.to_string()))
        }
    }

    /// Get file state snapshot.
    pub fn snapshot(&self, file_id: FileId) -> Option<DocumentState> {
        self.get(file_id).map(|s| s.get_state())
    }
}
