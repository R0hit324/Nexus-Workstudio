//! `nexus-protocol` defines the versioned wire protocol shared between the
//! Nexus client and server.
//!
//! Layering:
//!
//! ```text
//! Transport (abstract)
//!    ↓
//! Framing (length-prefixed frames)
//!    ↓
//! Serialization (compact binary)
//!    ↓
//! Message (typed protocol envelope)
//!    ↓
//! Application handler
//! ```
//!
//! Domain logic must depend on [`Transport`], never on TCP details directly.

pub mod frame;
pub mod message;
pub mod sync;
pub mod transport;

pub use frame::{FrameCodec, MAX_FRAME_LENGTH};
pub use message::{
    Capability, ErrorCode, EventRange, KnownCapability, Message, OperationReconciliationResult,
    OperationReconciliationStatus, PendingOperation, PluginRecordInfo, RecoverySessionStatus,
    RecoveryStage, SyncStrategy,
};
pub use sync::{EditOperation, OperationAck, OperationBatch, OperationId, TextOperation, Version};
pub use transport::{TcpTransport, Transport, TransportError};

use nexus_core::version::Version as CoreVersion;

/// The version of the wire protocol spoken by this crate.
pub const PROTOCOL_VERSION: CoreVersion = nexus_core::version::PROTOCOL_VERSION;
