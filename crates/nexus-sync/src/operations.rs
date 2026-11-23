use std::fmt;

use nexus_core::ids::{CubeId, FileId, UserId};
use nexus_protocol::sync::{
    EditOperation as ProtoEditOperation, OperationAck as ProtoOperationAck,
    OperationBatch as ProtoOperationBatch, OperationId as ProtoOperationId,
    TextOperation as ProtoTextOperation, Version as ProtoVersion,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for an edit operation.
///
/// Used for deduplication, acknowledgment, and retry.
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

impl fmt::Display for OperationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<OperationId> for ProtoOperationId {
    fn from(id: OperationId) -> Self {
        ProtoOperationId(id.0)
    }
}

impl From<ProtoOperationId> for OperationId {
    fn from(id: ProtoOperationId) -> Self {
        OperationId(id.0)
    }
}

/// Version/state identifier for a collaborative document.
///
/// Monotonically increases as operations are applied.
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

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Version> for ProtoVersion {
    fn from(v: Version) -> Self {
        ProtoVersion(v.0)
    }
}

impl From<ProtoVersion> for Version {
    fn from(v: ProtoVersion) -> Self {
        Version(v.0)
    }
}

/// Text operation types compatible with Yrs CRDT.
///
/// These map to Yrs's YText operations (insert/delete).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TextOperation {
    /// Insert text at position.
    Insert { position: u32, text: String },

    /// Delete range of text.
    Delete { position: u32, length: u32 },

    /// Retain (no-op, for future OT compatibility).
    Retain { length: u32 },
}

impl TextOperation {
    pub fn is_insert(&self) -> bool {
        matches!(self, TextOperation::Insert { .. })
    }

    pub fn is_delete(&self) -> bool {
        matches!(self, TextOperation::Delete { .. })
    }
}

impl From<TextOperation> for ProtoTextOperation {
    fn from(op: TextOperation) -> Self {
        match op {
            TextOperation::Insert { position, text } => {
                ProtoTextOperation::Insert { position, text }
            }
            TextOperation::Delete { position, length } => {
                ProtoTextOperation::Delete { position, length }
            }
            TextOperation::Retain { length } => ProtoTextOperation::Retain { length },
        }
    }
}

impl From<ProtoTextOperation> for TextOperation {
    fn from(op: ProtoTextOperation) -> Self {
        match op {
            ProtoTextOperation::Insert { position, text } => {
                TextOperation::Insert { position, text }
            }
            ProtoTextOperation::Delete { position, length } => {
                TextOperation::Delete { position, length }
            }
            ProtoTextOperation::Retain { length } => TextOperation::Retain { length },
        }
    }
}

/// Edit operation submitted by a client.
///
/// Contains all metadata needed for server validation and CRDT application.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditOperation {
    pub operation_id: OperationId,
    pub file_id: FileId,
    pub actor_id: UserId,
    pub base_version: Version,
    pub operations: Vec<TextOperation>,
    pub timestamp: u64,
}

impl EditOperation {
    pub fn new(
        file_id: FileId,
        actor_id: UserId,
        base_version: Version,
        operations: Vec<TextOperation>,
    ) -> Self {
        Self {
            operation_id: OperationId::new(),
            file_id,
            actor_id,
            base_version,
            operations,
            timestamp: nexus_core::timestamp::Timestamp::now().as_millis(),
        }
    }
}

impl From<EditOperation> for ProtoEditOperation {
    fn from(op: EditOperation) -> Self {
        ProtoEditOperation {
            operation_id: op.operation_id.into(),
            file_id: op.file_id,
            actor_id: op.actor_id,
            base_version: op.base_version.into(),
            operations: op.operations.into_iter().map(Into::into).collect(),
            timestamp: op.timestamp,
        }
    }
}

impl From<ProtoEditOperation> for EditOperation {
    fn from(op: ProtoEditOperation) -> Self {
        EditOperation {
            operation_id: op.operation_id.into(),
            file_id: op.file_id,
            actor_id: op.actor_id,
            base_version: op.base_version.into(),
            operations: op.operations.into_iter().map(Into::into).collect(),
            timestamp: op.timestamp,
        }
    }
}

/// Operation state on the client side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationStatus {
    /// Created locally, not yet sent.
    Pending,
    /// Sent to server, awaiting acknowledgment.
    Sent,
    /// Acknowledged by server (authoritative).
    Acknowledged,
    /// Rejected by server with reason.
    Rejected,
    /// Transformed by server (concurrent edit merged).
    Transformed,
    /// Applied to local document after server acknowledgment.
    Applied,
}

impl Default for OperationStatus {
    fn default() -> Self {
        Self::Pending
    }
}

/// Batch of edit operations for efficient network transmission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationBatch {
    pub cube_id: CubeId,
    pub operations: Vec<EditOperation>,
}

impl OperationBatch {
    pub fn new(cube_id: CubeId, operations: Vec<EditOperation>) -> Self {
        Self {
            cube_id,
            operations,
        }
    }

    pub fn len(&self) -> usize {
        self.operations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }
}

impl From<OperationBatch> for ProtoOperationBatch {
    fn from(batch: OperationBatch) -> Self {
        ProtoOperationBatch {
            cube_id: batch.cube_id,
            operations: batch.operations.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<ProtoOperationBatch> for OperationBatch {
    fn from(batch: ProtoOperationBatch) -> Self {
        OperationBatch {
            cube_id: batch.cube_id,
            operations: batch.operations.into_iter().map(Into::into).collect(),
        }
    }
}

/// Server acknowledgment for an operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationAck {
    pub operation_id: OperationId,
    pub version: Version,
    pub accepted: bool,
    pub rejection_reason: Option<String>,
    pub transformed_operations: Option<Vec<TextOperation>>,
}

impl From<OperationAck> for ProtoOperationAck {
    fn from(ack: OperationAck) -> Self {
        ProtoOperationAck {
            operation_id: ack.operation_id.into(),
            version: ack.version.into(),
            accepted: ack.accepted,
            rejection_reason: ack.rejection_reason,
            transformed_operations: ack
                .transformed_operations
                .map(|v| v.into_iter().map(Into::into).collect()),
        }
    }
}

impl From<ProtoOperationAck> for OperationAck {
    fn from(ack: ProtoOperationAck) -> Self {
        OperationAck {
            operation_id: ack.operation_id.into(),
            version: ack.version.into(),
            accepted: ack.accepted,
            rejection_reason: ack.rejection_reason,
            transformed_operations: ack
                .transformed_operations
                .map(|v| v.into_iter().map(Into::into).collect()),
        }
    }
}
