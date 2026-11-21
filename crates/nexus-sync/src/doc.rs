use std::sync::Arc;

use nexus_core::ids::{FileId, UserId};
use parking_lot::RwLock;

use crate::{
    cursor::SyncStatus,
    engine::TextSyncEngine,
    error::SyncResult,
    operations::{EditOperation, OperationId, TextOperation, Version},
};

/// State of a collaborative document.
#[derive(Debug, Clone)]
pub struct DocumentState {
    pub file_id: FileId,
    pub version: Version,
    pub status: SyncStatus,
    pub content: String,
    pub pending_operations: Vec<EditOperation>,
    pub last_synced: Option<u64>,
}

impl DocumentState {
    pub fn new(file_id: FileId) -> Self {
        Self {
            file_id,
            version: Version::INITIAL,
            status: SyncStatus::Unknown,
            content: String::new(),
            pending_operations: Vec::new(),
            last_synced: None,
        }
    }

    pub fn is_synchronized(&self) -> bool {
        matches!(self.status, SyncStatus::Synchronized)
    }
}

/// Core abstraction for a collaborative document.
///
/// Isolates the CRDT/OT algorithm from the editor, network, and storage layers.
pub trait CollaborationDocument: Send + Sync {
    /// Apply a local edit operation optimistically.
    fn apply_local(&self, operation: EditOperation) -> SyncResult<Vec<TextOperation>>;

    /// Apply a remote operation from the server/other clients.
    fn apply_remote(&self, operation: EditOperation) -> SyncResult<()>;

    /// Generate an operation from a text change.
    fn generate_operation(
        &self,
        actor_id: UserId,
        base_version: Version,
        ops: Vec<TextOperation>,
    ) -> EditOperation;

    /// Acknowledge an operation (server confirmed).
    fn acknowledge(&self, operation_id: OperationId, version: Version) -> SyncResult<()>;

    /// Get the current document version.
    fn version(&self) -> Version;

    /// Get the current document content as plain text.
    fn content(&self) -> String;

    /// Get synchronization status.
    fn status(&self) -> SyncStatus;

    /// Get pending local operations.
    fn pending_operations(&self) -> Vec<EditOperation>;

    /// Check if document has unacknowledged changes.
    fn has_pending_changes(&self) -> bool;

    /// Request delta synchronization from a version.
    fn request_delta(&self, from_version: Version) -> SyncResult<Vec<EditOperation>>;

    /// Force resynchronization (e.g., after desynchronization).
    fn resynchronize(&self, content: String, version: Version) -> SyncResult<()>;
}

/// Default implementation using Yrs CRDT engine.
pub struct YrsDocument {
    engine: Arc<dyn TextSyncEngine>,
    file_id: FileId,
    version: RwLock<Version>,
    status: RwLock<SyncStatus>,
    pending: RwLock<Vec<EditOperation>>,
    content_cache: RwLock<String>,
}

impl YrsDocument {
    pub fn new(engine: Arc<dyn TextSyncEngine>, file_id: FileId) -> Self {
        Self {
            engine,
            file_id,
            version: RwLock::new(Version::INITIAL),
            status: RwLock::new(SyncStatus::Unknown),
            pending: RwLock::new(Vec::new()),
            content_cache: RwLock::new(String::new()),
        }
    }

    fn update_content_cache(&self) {
        let content = self.engine.content();
        *self.content_cache.write() = content;
    }
}

impl CollaborationDocument for YrsDocument {
    fn apply_local(&self, operation: EditOperation) -> SyncResult<Vec<TextOperation>> {
        let ops = self.engine.local_edit(&operation)?;
        let mut pending = self.pending.write();
        pending.push(operation);
        self.update_content_cache();
        Ok(ops)
    }

    fn apply_remote(&self, operation: EditOperation) -> SyncResult<()> {
        self.engine.remote_edit(&operation)?;
        *self.version.write() = operation.base_version.next();
        self.update_content_cache();
        Ok(())
    }

    fn generate_operation(
        &self,
        actor_id: UserId,
        base_version: Version,
        ops: Vec<TextOperation>,
    ) -> EditOperation {
        EditOperation::new(self.file_id, actor_id, base_version, ops)
    }

    fn acknowledge(&self, operation_id: OperationId, version: Version) -> SyncResult<()> {
        let mut pending = self.pending.write();
        pending.retain(|op| op.operation_id != operation_id);
        *self.version.write() = version;
        self.update_content_cache();
        Ok(())
    }

    fn version(&self) -> Version {
        *self.version.read()
    }

    fn content(&self) -> String {
        self.content_cache.read().clone()
    }

    fn status(&self) -> SyncStatus {
        *self.status.read()
    }

    fn pending_operations(&self) -> Vec<EditOperation> {
        self.pending.read().clone()
    }

    fn has_pending_changes(&self) -> bool {
        !self.pending.read().is_empty()
    }

    fn request_delta(&self, from_version: Version) -> SyncResult<Vec<EditOperation>> {
        self.engine.request_delta(from_version)
    }

    fn resynchronize(&self, content: String, version: Version) -> SyncResult<()> {
        self.engine.resynchronize(content, version)?;
        *self.version.write() = version;
        *self.status.write() = SyncStatus::Synchronized;
        self.update_content_cache();
        Ok(())
    }
}
