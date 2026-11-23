//! Protocol message types for Part 5 Real-Time Synchronization.
//!
//! These are the wire-format types for synchronization. The nexus-sync crate
//! uses these types for its internal implementation.

use nexus_core::ids::{CubeId, FileId, UserId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Version/state identifier for a collaborative document.
///
/// Uses u64 for larger version space in collaborative editing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Version(pub u64);

impl Version {
    pub const INITIAL: Self = Self(0);
    pub fn next(&self) -> Self {
        Self(self.0 + 1)
    }
    pub fn value(&self) -> u64 {
        self.0
    }
}

/// Unique identifier for an edit operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OperationId(pub Uuid);

impl OperationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for OperationId {
    fn default() -> Self {
        Self::new()
    }
}

/// Text operation types for CRDT-based editing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TextOperation {
    Insert { position: u32, text: String },
    Delete { position: u32, length: u32 },
    Retain { length: u32 },
}

/// Edit operation submitted by a client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditOperation {
    pub operation_id: OperationId,
    pub file_id: FileId,
    pub actor_id: UserId,
    pub base_version: Version,
    pub operations: Vec<TextOperation>,
    pub timestamp: u64,
}

/// Server acknowledgment for an operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperationAck {
    pub operation_id: OperationId,
    pub version: Version,
    pub accepted: bool,
    pub rejection_reason: Option<String>,
    pub transformed_operations: Option<Vec<TextOperation>>,
}

/// Batch of edit operations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperationBatch {
    pub cube_id: CubeId,
    pub operations: Vec<EditOperation>,
}

/// Cursor position/update (transient, high-frequency).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CursorUpdate {
    pub user_id: UserId,
    pub file_id: FileId,
    pub position: u32,
    pub selection_start: Option<u32>,
    pub selection_end: Option<u32>,
    pub timestamp: u64,
}

/// Presence update (user status, active file).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresenceUpdate {
    pub user_id: UserId,
    pub cube_id: CubeId,
    pub status: Option<PresenceStatus>,
    pub current_file: Option<FileId>,
    pub cursor: Option<CursorUpdate>,
    pub timestamp: u64,
}

/// User presence status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresenceStatus {
    Active,
    Idle,
    Away,
    Offline,
}

/// Synchronization status for a document or file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncStatus {
    Unknown,
    Loading,
    Synchronizing,
    Synchronized,
    Desynchronized,
    Recovering,
    Offline,
}

/// Protocol version for sync messages.
pub const SYNC_PROTOCOL_VERSION: u8 = 1;
