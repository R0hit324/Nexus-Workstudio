use std::collections::HashMap;

use nexus_core::ids::{CubeId, FileId, SessionId, UserId};
use parking_lot::RwLock;

use crate::{
    cursor::{CursorUpdate, PresenceManager, PresenceUpdate, SyncStatus},
    doc::DocumentState,
    error::{SyncError, SyncResult},
    file_sync::FileSyncManager,
    operations::{EditOperation, OperationAck, OperationId, OperationStatus, Version},
};

/// Client-side synchronization configuration.
#[derive(Debug, Clone)]
pub struct ClientSyncConfig {
    pub cursor_throttle_ms: u64,
    pub max_pending_operations: usize,
    pub auto_reconnect: bool,
    pub delta_sync_threshold: u64,
}

impl Default for ClientSyncConfig {
    fn default() -> Self {
        Self {
            cursor_throttle_ms: 50,
            max_pending_operations: 500,
            auto_reconnect: true,
            delta_sync_threshold: 1000,
        }
    }
}

/// Client-side synchronization manager for a Cube.
pub struct ClientSyncManager {
    #[allow(dead_code)]
    cube_id: CubeId,
    config: ClientSyncConfig,
    user_id: UserId,
    #[allow(dead_code)]
    session_id: SessionId,
    file_manager: FileSyncManager,
    presence: PresenceManager,
    pending_operations: RwLock<HashMap<OperationId, (EditOperation, OperationStatus)>>,
    local_version: RwLock<Version>,
    sync_status: RwLock<SyncStatus>,
}

impl ClientSyncManager {
    pub fn new(
        cube_id: CubeId,
        config: ClientSyncConfig,
        user_id: UserId,
        session_id: SessionId,
    ) -> Self {
        let cursor_throttle = config.cursor_throttle_ms;
        Self {
            cube_id,
            config,
            user_id,
            session_id,
            file_manager: FileSyncManager::new(cube_id),
            presence: PresenceManager::new(cube_id)
                .with_throttle(std::time::Duration::from_millis(cursor_throttle)),
            pending_operations: RwLock::new(HashMap::new()),
            local_version: RwLock::new(Version::INITIAL),
            sync_status: RwLock::new(SyncStatus::Unknown),
        }
    }

    /// Create a local edit operation and apply optimistically.
    pub fn apply_local_edit(
        &self,
        file_id: FileId,
        operations: Vec<crate::operations::TextOperation>,
    ) -> SyncResult<EditOperation> {
        let file_state = self.file_manager.get_or_create(file_id);
        let base_version = *self.local_version.read();

        let operation = EditOperation::new(file_id, self.user_id, base_version, operations);

        // Apply locally (optimistic)
        file_state.apply_local(operation.clone())?;

        // Track as pending
        let mut pending = self.pending_operations.write();
        if pending.len() >= self.config.max_pending_operations {
            return Err(SyncError::Internal("too many pending operations".into()));
        }
        pending.insert(
            operation.operation_id,
            (operation.clone(), OperationStatus::Pending),
        );

        *self.sync_status.write() = SyncStatus::Synchronizing;

        Ok(operation)
    }

    /// Handle server acknowledgment.
    pub fn handle_ack(&self, ack: OperationAck) -> SyncResult<()> {
        let mut pending = self.pending_operations.write();

        if let Some((mut operation, _)) = pending.remove(&ack.operation_id) {
            if ack.accepted {
                operation.base_version = ack.version;
                // Apply to local document
                let file_state = self.file_manager.get_or_create(operation.file_id);
                file_state.acknowledge(ack.operation_id, ack.version)?;
                *self.local_version.write() = ack.version;
                *self.sync_status.write() = SyncStatus::Synchronized;
            } else {
                // Operation rejected - need to reconcile
                *self.sync_status.write() = SyncStatus::Desynchronized;
                // In a full implementation, we'd handle rejection/retry here
            }
        }

        Ok(())
    }

    /// Handle batch acknowledgment.
    pub fn handle_batch_ack(&self, acks: Vec<OperationAck>) -> SyncResult<()> {
        for ack in acks {
            self.handle_ack(ack)?;
        }
        Ok(())
    }

    /// Handle remote operation from server/other clients.
    pub fn handle_remote_operation(&self, operation: EditOperation) -> SyncResult<()> {
        let file_state = self.file_manager.get_or_create(operation.file_id);
        file_state.apply_remote(operation.clone())?;
        *self.local_version.write() = file_state.version();
        *self.sync_status.write() = SyncStatus::Synchronized;
        Ok(())
    }

    /// Handle batch of remote operations.
    pub fn handle_remote_batch(&self, batch: crate::operations::OperationBatch) -> SyncResult<()> {
        for operation in batch.operations {
            self.handle_remote_operation(operation)?;
        }
        Ok(())
    }

    /// Send cursor update.
    pub fn send_cursor(&self, cursor: CursorUpdate) {
        self.presence.update_cursor(cursor);
    }

    /// Send presence update.
    pub fn send_presence(&self, update: PresenceUpdate) {
        self.presence.update_presence(update);
    }

    /// Register user joined.
    pub fn user_joined(&self, user_id: UserId, username: String) -> crate::cursor::UserColor {
        self.presence.upsert_user(user_id, username)
    }

    /// Register user left.
    pub fn user_left(&self, user_id: UserId) {
        self.presence.remove_user(user_id);
    }

    /// Open a file.
    pub fn open_file(&self, file_id: FileId) -> SyncResult<DocumentState> {
        let state = self.file_manager.get_or_create(file_id);
        Ok(state.get_state())
    }

    /// Close a file.
    pub fn close_file(&self, _file_id: FileId) {
        // Local presence update handled by UI
    }

    /// Get file state.
    pub fn get_file_state(&self, file_id: FileId) -> Option<DocumentState> {
        self.file_manager.snapshot(file_id)
    }

    /// Get current sync status.
    pub fn sync_status(&self) -> SyncStatus {
        *self.sync_status.read()
    }

    /// Get local version.
    pub fn local_version(&self) -> Version {
        *self.local_version.read()
    }

    /// Get pending operations.
    pub fn pending_operations(&self) -> Vec<EditOperation> {
        self.pending_operations
            .read()
            .values()
            .map(|(op, _)| op.clone())
            .collect()
    }

    /// Check if there are unacknowledged operations.
    pub fn has_pending(&self) -> bool {
        !self.pending_operations.read().is_empty()
    }

    /// Request delta synchronization.
    pub fn request_delta(
        &self,
        file_id: FileId,
        from_version: Version,
    ) -> SyncResult<Vec<EditOperation>> {
        let file_state = self.file_manager.get_or_create(file_id);
        file_state.document().request_delta(from_version)
    }

    /// Handle delta response from server.
    pub fn handle_delta(&self, file_id: FileId, operations: Vec<EditOperation>) -> SyncResult<()> {
        let file_state = self.file_manager.get_or_create(file_id);
        for operation in operations {
            file_state.apply_remote(operation)?;
        }
        *self.local_version.write() = file_state.version();
        *self.sync_status.write() = SyncStatus::Synchronized;
        Ok(())
    }

    /// Force resynchronization (e.g., after desynchronization).
    pub fn resynchronize(
        &self,
        file_id: FileId,
        content: String,
        version: Version,
    ) -> SyncResult<()> {
        let file_state = self.file_manager.get_or_create(file_id);
        file_state.resync(content, version)?;
        *self.local_version.write() = version;
        *self.sync_status.write() = SyncStatus::Synchronized;
        Ok(())
    }

    /// Create a new file.
    pub fn create_file(&self, file_id: FileId, initial_content: String) -> SyncResult<()> {
        self.file_manager
            .create_with_content(file_id, initial_content);
        Ok(())
    }

    /// Delete a file.
    pub fn delete_file(&self, file_id: FileId) -> SyncResult<()> {
        self.file_manager.remove(file_id);
        Ok(())
    }

    /// Rename a file.
    pub fn rename_file(&self, old_id: FileId, new_id: FileId) -> SyncResult<()> {
        self.file_manager.rename(old_id, new_id);
        Ok(())
    }

    /// Get all users in cube.
    pub fn get_users(&self) -> Vec<crate::cursor::PresenceState> {
        self.presence.all_users()
    }

    /// Get users in a specific file.
    pub fn get_users_in_file(&self, file_id: FileId) -> Vec<crate::cursor::PresenceState> {
        self.presence.users_in_file(file_id)
    }
}
