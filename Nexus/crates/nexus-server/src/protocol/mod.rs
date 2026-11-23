//! Protocol-facing server logic: connection handshake and request dispatch.

pub mod dispatcher;
pub mod handshake;

pub use dispatcher::{error, to_protocol_error, DispatchAction, DispatchContext, Dispatcher};
pub use handshake::{capabilities_list, capability_intersection, perform_handshake, Handshake};
