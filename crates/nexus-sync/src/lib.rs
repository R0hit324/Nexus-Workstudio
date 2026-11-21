//! Part 5: Real-Time Synchronization Engine
//!
//! CRDT-based collaborative editing with file-level synchronization,
//! cursor/presence awareness, and operation management.

pub mod client;
pub mod cursor;
pub mod doc;
pub mod engine;
pub mod error;
pub mod file_sync;
pub mod metrics;
pub mod operations;
pub mod server;

// Re-export protocol sync types
pub use nexus_protocol::sync::{
    CursorUpdate as ProtoCursorUpdate, EditOperation as ProtoEditOperation,
    OperationAck as ProtoOperationAck, OperationBatch as ProtoOperationBatch,
    OperationId as ProtoOperationId, PresenceStatus as ProtoPresenceStatus,
    PresenceUpdate as ProtoPresenceUpdate, SyncStatus as ProtoSyncStatus,
    TextOperation as ProtoTextOperation, Version as ProtoVersion,
};

pub use client::{ClientSyncConfig, ClientSyncManager};
pub use cursor::{
    CursorUpdate, PresenceManager, PresenceState, PresenceStatus, PresenceUpdate, SyncStatus,
    UserColor,
};
pub use doc::{CollaborationDocument, DocumentState, YrsDocument};
pub use engine::{SyncEngineFactory, TextSyncEngine, YrsSyncEngine};
pub use error::{SyncError, SyncResult};
pub use file_sync::{FileSyncManager, FileSyncState};
pub use metrics::{SyncMetrics, SyncMetricsSnapshot};
pub use nexus_core::ids::FileId;
pub use operations::{
    EditOperation, OperationAck, OperationBatch, OperationId, OperationStatus, TextOperation,
    Version,
};
pub use server::{ServerSyncConfig, ServerSyncManager};

/// Part 5 phase identifier for runtime inspection.
pub const PHASE: &str = "part5-realtime-sync";
