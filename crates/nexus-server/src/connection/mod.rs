//! Network connection bookkeeping: registration, routing and bounded
//! outbound queues with explicit backpressure policies.

pub mod manager;

pub use manager::{ConnectionManager, Delivery, SendOutcome};
