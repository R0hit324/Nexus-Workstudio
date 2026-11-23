use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use nexus_core::ids::{CubeId, FileId, SessionId, UserId};
use nexus_events::bus::EventBus;
use parking_lot::RwLock;

use crate::{
    cursor::{CursorUpdate, PresenceManager, PresenceState, PresenceUpdate, UserColor},
    error::{SyncError, SyncResult},
    file_sync::FileSyncManager,
    operations::{EditOperation, OperationAck, OperationBatch, Version},
};

/// Server-side synchronization configuration.
#[derive(Debug, Clone)]
pub struct ServerSyncConfig {
    pub max_operation_size: usize,
    pub max_pending_operations: usize,
    pub cursor_throttle_ms: u64,
    pub presence_timeout_secs: u64,
    pub rate_limit_ops_per_sec: u32,
}

impl Default for ServerSyncConfig {
    fn default() -> Self {
        Self {
            max_operation_size: 1024 * 1024, // 1 MB
            max_pending_operations: 1000,
            cursor_throttle_ms: 50,
            presence_timeout_secs: 300,
            rate_limit_ops_per_sec: 100,
        }
    }
}

/// Per-session rate limiting state.
struct RateLimiter {
    operations_this_second: u32,
    window_start: Instant,
    limit: u32,
}

impl RateLimiter {
    fn new(limit: u32) -> Self {
        Self {
            operations_this_second: 0,
            window_start: Instant::now(),
            limit,
        }
    }

    fn check(&mut self) -> bool {
        let now = Instant::now();
        if now.duration_since(self.window_start).as_secs() >= 1 {
            self.operations_this_second = 0;
            self.window_start = now;
        }
        if self.operations_this_second >= self.limit {
            false
        } else {
            self.operations_this_second += 1;
            true
        }
    }
}

/// Server-side synchronization manager for a Cube.
pub struct ServerSyncManager {
    cube_id: CubeId,
    config: ServerSyncConfig,
    file_manager: FileSyncManager,
    presence: PresenceManager,
    #[allow(dead_code)]
    event_bus: Arc<EventBus>,
    rate_limiters: RwLock<HashMap<SessionId, RateLimiter>>,
}

impl ServerSyncManager {
    pub fn new(cube_id: CubeId, config: ServerSyncConfig, event_bus: Arc<EventBus>) -> Self {
        let cursor_throttle = config.cursor_throttle_ms;
        Self {
            cube_id,
            config,
            file_manager: FileSyncManager::new(cube_id),
            presence: PresenceManager::new(cube_id)
                .with_throttle(std::time::Duration::from_millis(cursor_throttle)),
            event_bus,
            rate_limiters: RwLock::new(HashMap::new()),
        }
    }

    fn check_rate_limit(&self, session_id: SessionId) -> SyncResult<()> {
        let mut limiters = self.rate_limiters.write();
        let limiter = limiters
            .entry(session_id)
            .or_insert_with(|| RateLimiter::new(self.config.rate_limit_ops_per_sec));
        if limiter.check() {
            Ok(())
        } else {
            Err(SyncError::RateLimited(
                "operation rate limit exceeded".into(),
            ))
        }
    }

    /// Validate operation size.
    fn validate_operation(&self, operation: &EditOperation) -> SyncResult<()> {
        let size = serde_json::to_vec(operation).map(|v| v.len()).unwrap_or(0);
        if size > self.config.max_operation_size {
            return Err(SyncError::OperationTooLarge {
                size,
                max: self.config.max_operation_size,
            });
        }
        Ok(())
    }

    /// Process an edit operation from a client.
    pub fn process_operation(
        &self,
        session_id: SessionId,
        _user_id: UserId,
        operation: EditOperation,
    ) -> SyncResult<OperationAck> {
        // Rate limiting
        self.check_rate_limit(session_id)?;

        // Validate operation
        self.validate_operation(&operation)?;

        // TODO: Check permissions (user can edit this file in this cube)
        // For now, assume authorized

        // Apply to file sync state
        let file_state = self.file_manager.get_or_create(operation.file_id);

        // Check version - if client version is stale, we need to transform
        let current_version = file_state.version();
        if operation.base_version < current_version {
            // Client is behind - in a full implementation we'd transform
            // For now, reject with version mismatch
            return Ok(OperationAck {
                operation_id: operation.operation_id,
                version: current_version,
                accepted: false,
                rejection_reason: Some(format!(
                    "stale version: client {}, server {}",
                    operation.base_version, current_version
                )),
                transformed_operations: None,
            });
        }

        // Apply the operation
        file_state.apply_remote(operation.clone())?;

        // Create acknowledgment
        let new_version = file_state.version();
        let ack = OperationAck {
            operation_id: operation.operation_id,
            version: new_version,
            accepted: true,
            rejection_reason: None,
            transformed_operations: None,
        };

        // Emit event for other subscribers (via event bus)
        self.emit_file_edited(&operation, new_version)?;

        Ok(ack)
    }

    /// Process a batch of operations.
    pub fn process_batch(
        &self,
        session_id: SessionId,
        user_id: UserId,
        batch: OperationBatch,
    ) -> SyncResult<Vec<OperationAck>> {
        let mut acks = Vec::with_capacity(batch.operations.len());
        for operation in batch.operations {
            let ack = self.process_operation(session_id, user_id, operation)?;
            acks.push(ack);
        }
        Ok(acks)
    }

    /// Handle cursor update from client.
    pub fn handle_cursor(&self, _session_id: SessionId, _user_id: UserId, cursor: CursorUpdate) {
        self.presence.update_cursor(cursor);
    }

    /// Handle presence update from client.
    pub fn handle_presence(&self, _user_id: UserId, update: PresenceUpdate) {
        self.presence.update_presence(update);
    }

    /// Register a user joining the cube.
    pub fn user_joined(&self, user_id: UserId, username: String) -> UserColor {
        self.presence.upsert_user(user_id, username)
    }

    /// Register a user leaving the cube.
    pub fn user_left(&self, user_id: UserId) {
        self.presence.remove_user(user_id);
    }

    /// Open a file for a user.
    pub fn open_file(
        &self,
        user_id: UserId,
        file_id: FileId,
    ) -> SyncResult<crate::doc::DocumentState> {
        let state = self.file_manager.get_or_create(file_id);
        self.presence.set_user_file(user_id, Some(file_id));
        Ok(state.get_state())
    }

    /// Close a file for a user.
    pub fn close_file(&self, user_id: UserId, _file_id: FileId) {
        self.presence.set_user_file(user_id, None);
    }

    /// Get file state.
    pub fn get_file_state(&self, file_id: FileId) -> Option<crate::doc::DocumentState> {
        self.file_manager.snapshot(file_id)
    }

    /// Get all users in cube.
    pub fn get_users(&self) -> Vec<PresenceState> {
        self.presence.all_users()
    }

    /// Get users in a specific file.
    pub fn get_users_in_file(&self, file_id: FileId) -> Vec<PresenceState> {
        self.presence.users_in_file(file_id)
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

    /// Emit file edited event to event bus.
    fn emit_file_edited(&self, operation: &EditOperation, version: Version) -> SyncResult<()> {
        // Create a FileEdited event and publish to event bus
        // This integrates with Part 4 event system
        use nexus_core::event::{Event, EventPayload};

        let event = Event::new(
            self.cube_id,
            operation.actor_id,
            EventPayload::FileEdited {
                path: operation.file_id.to_string(),
                version: nexus_core::version::Version::new(version.value() as u32),
            },
        );

        // Note: This would need the event to be sequenced by the event bus
        // For now, we skip actual event emission to avoid circular deps
        // The server's dispatcher would handle this
        let _ = event;
        let _ = version;
        Ok(())
    }
}
