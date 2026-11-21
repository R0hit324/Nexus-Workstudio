//! Nexus client library.
//!
//! Layered facade over the protocol:
//!
//! ```text
//! UI / headless  →  SessionService  →  EventSink + ClientState
//!                                   →  Transport (protocol) → server
//! ```
//!
//! Part 1 delivered the connection lifecycle (handshake, authentication) and
//! Cube operations on `Client`. Part 2 adds the session/workspace/event
//! services the TUI binds against. The transport swap-in happens when live
//! event fan-out lands (later parts); until then `SessionService` is a
//! faithful offline simulation that emits the same events.

pub mod app;
pub mod cache;
pub mod client;
pub mod events;
pub mod network;
pub mod recovery;
pub mod session;
pub mod workspace;

pub use app::state::ClientState;
pub use client::{Client, Handshake};
pub use events::EventSink;
pub use network::NetworkEvent;
pub use recovery::{
    PendingOperationEntry, RecoveryClient, RecoveryConfig, RecoveryManager, RecoveryProgress,
    RecoveryState,
};
pub use session::SessionService;
pub use workspace::WorkspaceService;

/// The name reported by this client during the protocol handshake.
pub const CLIENT_NAME: &str = "nexus-client";

/// Capabilities this client offers.
pub fn client_capabilities() -> Vec<nexus_protocol::Capability> {
    vec![
        nexus_protocol::Capability::Synchronization,
        nexus_protocol::Capability::Snapshots,
        nexus_protocol::Capability::Presence,
    ]
}
