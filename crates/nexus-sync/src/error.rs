use thiserror::Error;

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("document not found: {0}")]
    DocumentNotFound(String),

    #[error("operation rejected: {0}")]
    OperationRejected(String),

    #[error("invalid operation: {0}")]
    InvalidOperation(String),

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("version mismatch: expected {expected}, got {actual}")]
    VersionMismatch { expected: u64, actual: u64 },

    #[error("file not found: {0}")]
    FileNotFound(String),

    #[error("desynchronized: {0}")]
    Desynchronized(String),

    #[error("operation too large: {size} bytes (max {max})")]
    OperationTooLarge { size: usize, max: usize },

    #[error("rate limited: {0}")]
    RateLimited(String),

    #[error("encoding error: {0}")]
    EncodingError(#[from] serde_json::Error),

    #[error("CRDT error: {0}")]
    CrdtError(String),

    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("protocol error: {0}")]
    ProtocolError(String),

    #[error("event error: {0}")]
    EventError(#[from] nexus_events::Error),

    #[error("core error: {0}")]
    CoreError(#[from] nexus_core::error::Error),

    #[error("internal error: {0}")]
    Internal(String),
}

pub type SyncResult<T> = Result<T, SyncError>;
