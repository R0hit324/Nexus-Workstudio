//! # Part 4 — Event System
//!
//! A production-oriented event backbone for Nexus.
//!
//! ## Pipeline
//!
//! ```text
//!   publish(event)
//!       │  validate + assign per-Cube sequence      (sequencer, no global lock)
//!       ▼
//!   persist (durable/reliable only)                 (EventStore)
//!       │
//!       ▼
//!   fan-out to internal subscribers                 (EventBus, isolation + backpressure)
//!       │
//!       ▼
//!   consumers / projections / checkpoints           (ConsumerRegistry, Projector)
//! ```
//!
//! Design invariants (spec section 5):
//! * **Immutability** — events are never mutated after construction;
//!   `with_sequence`/`with_session` produce new events.
//! * **Per-Cube ordering** — sequences are assigned monotonically per Cube
//!   with no global server lock, so independent Cubes make progress in
//!   parallel.
//! * **Isolation** — a slow or broken subscriber never blocks publishing;
//!   each subscription owns a bounded queue and is drained at its own pace
//!   (spec section 32).
//! * **Classification** — every event type has a category and a delivery
//!   class; delivery class decides persistence and drop-vs-backpressure
//!   behavior.

pub mod ack;
pub mod batch;
pub mod bus;
pub mod consumer;
pub mod convert;
pub mod filter;
pub mod metrics;
pub mod projection;
pub mod registry;
pub mod replay;
pub mod security;
pub mod seq;
pub mod store;
pub mod trace;

/// The milestone this crate contains.
pub const PHASE: &str = "part4-event-system";

/// Common error type for the event engine.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("unknown schema version {version}; expected {expected}")]
    UnknownSchema { version: u32, expected: u32 },
    #[error("invalid cube id received")]
    InvalidCube,
    #[error("event already has a sequence assigned")]
    AlreadySequenced,
    #[error("out-of-order/duplicate append: expected seq {expected}, got {got}")]
    SequenceMismatch { expected: u64, got: u64 },
    #[error("replay gap detected at sequence {0}")]
    ReplayGap(nexus_core::event::EventSequence),
    #[error("unexpected event type {0:?}")]
    UnexpectedEventType(String),
    #[error("event filtered out by subscription policy")]
    FilteredOut,
    #[error("invalid payload for event type {0:?}")]
    InvalidPayload(String),
}

impl Error {
    /// Shorthand for a serialization error.
    pub fn serde(err: impl std::fmt::Display) -> Self {
        Self::Serialization(err.to_string())
    }
}

pub(crate) type Result<T, E = Error> = std::result::Result<T, E>;

#[cfg(test)]
mod smoke_tests {
    use nexus_core::event::{Event, EventPayload};
    use nexus_core::ids::{CubeId, UserId};

    use crate::PHASE;

    #[test]
    fn crate_milestone_is_part4() {
        assert_eq!(PHASE, "part4-event-system");
    }

    #[test]
    fn basic_event_roundtrip() {
        let cube = CubeId::new();
        let actor = UserId::new();
        let event = Event::new(
            cube,
            actor,
            EventPayload::FileCreated {
                path: "/app/lib.rs".into(),
            },
        );
        assert!(!event.is_sequenced());
        assert_eq!(
            event.payload().clone(),
            EventPayload::FileCreated {
                path: "/app/lib.rs".into()
            }
        );
    }
}
