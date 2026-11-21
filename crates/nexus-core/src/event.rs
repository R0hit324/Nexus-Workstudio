use serde::{Deserialize, Serialize};

use crate::ids::{CubeId, EventId, SessionId, UserId};
use crate::timestamp::Timestamp;
use crate::version::Version;

/// The schema version of the [`Event`] record. Bump when the structure
/// changes; decoders must reject (or migrate) events with unknown schema
/// versions rather than guess.
pub const EVENT_SCHEMA_VERSION: u32 = 1;

/// A strictly ordered, per-Cube event sequence number.
///
/// Sequences are authoritative per Cube (spec: per-Cube ordering, never a
/// global server sequence). `EventSequence(0)` means "not yet assigned".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EventSequence(pub u64);

impl EventSequence {
    /// The un-assigned placeholder used before the sequencer stamps an event.
    pub const UNASSIGNED: Self = Self(0);

    /// Builds a sequence from a 64-bit counter.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// The underlying counter value.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }

    /// Returns the successor sequence.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

impl std::fmt::Display for EventSequence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u64> for EventSequence {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

/// Broad category of an event. Categories let subscribers request only the
/// topics they care about without enumerating every concrete event type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventCategory {
    Workspace,
    Collaboration,
    Session,
    Cube,
    Terminal,
    Build,
    Runtime,
    Snapshot,
    Plugin,
    Ai,
    System,
    Security,
}

/// Delivery semantics of an event. Events in different classes tolerate
/// different loss/backpressure behavior (spec section 31/32).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeliveryClass {
    /// Must be persisted and replayed (FileEdited, PermissionChanged, ...).
    Durable,
    /// Usually delivery, acknowledged; may be re-requested on gaps.
    Reliable,
    /// Delivery attempted; a slow subscriber may miss it without harm.
    BestEffort,
    /// High-frequency, non-durable (CursorMoved, SelectionChanged).
    Transient,
}

impl DeliveryClass {
    /// Whether events of this class are persisted to the event store.
    #[must_use]
    pub const fn is_persistent(self) -> bool {
        matches!(self, Self::Durable | Self::Reliable)
    }
}

/// The kind of an event (spec section 11). The set is intentionally groupable
/// into [`EventCategory`]s and extensible through the registry; the engine in
/// `nexus-events` owns the authoritative mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventType {
    // Cube lifecycle
    CubeCreated,
    CubeDeleted,
    // Collaboration / presence
    UserJoined,
    UserLeft,
    SelectionChanged,
    // Workspace
    FileCreated,
    FileDeleted,
    FileRenamed,
    FileEdited,
    CursorMoved,
    ChatMessageSent,
    // Session
    SessionTerminated,
    // Security
    AuthenticationSucceeded,
    AuthenticationFailed,
    UnauthorizedOperation,
    PermissionChanged,
    // Terminal / build
    TerminalStarted,
    TerminalInput,
    TerminalOutput,
    BuildStarted,
    BuildFinished,
    // Snapshots
    SnapshotCreated,
    // System (internal plumbing)
    StorageWriteCompleted,
    System,
}

impl EventType {
    /// The broad category this event type belongs to.
    #[must_use]
    pub const fn category(self) -> EventCategory {
        use EventCategory as C;
        use EventType::*;
        match self {
            CubeCreated | CubeDeleted => C::Cube,
            UserJoined | UserLeft | SelectionChanged | ChatMessageSent => C::Collaboration,
            FileCreated | FileDeleted | FileRenamed | FileEdited | CursorMoved => C::Workspace,
            SessionTerminated => C::Session,
            AuthenticationSucceeded
            | AuthenticationFailed
            | UnauthorizedOperation
            | PermissionChanged => C::Security,
            TerminalStarted | TerminalInput | TerminalOutput => C::Terminal,
            BuildStarted | BuildFinished => C::Build,
            SnapshotCreated => C::Snapshot,
            StorageWriteCompleted | System => C::System,
        }
    }

    /// The delivery guarantees attached to this event type.
    #[must_use]
    pub const fn delivery(self) -> DeliveryClass {
        use DeliveryClass as D;
        use EventType::*;
        match self {
            CubeCreated
            | CubeDeleted
            | FileCreated
            | FileDeleted
            | FileRenamed
            | FileEdited
            | PermissionChanged
            | AuthenticationSucceeded
            | AuthenticationFailed
            | UnauthorizedOperation
            | SessionTerminated => D::Durable,
            BuildFinished => D::BestEffort,
            UserJoined | UserLeft | BuildStarted | SnapshotCreated => D::BestEffort,
            ChatMessageSent => D::Reliable,
            TerminalStarted | TerminalInput | TerminalOutput | StorageWriteCompleted => {
                D::BestEffort
            }
            CursorMoved | SelectionChanged => D::Transient,
            System => D::BestEffort,
        }
    }

    /// Stable lowercase machine name (serialization + protocol integration).
    #[must_use]
    pub const fn name(self) -> &'static str {
        use EventType::*;
        match self {
            CubeCreated => "cube-created",
            CubeDeleted => "cube-deleted",
            UserJoined => "user-joined",
            UserLeft => "user-left",
            SelectionChanged => "selection-changed",
            FileCreated => "file-created",
            FileDeleted => "file-deleted",
            FileRenamed => "file-renamed",
            FileEdited => "file-edited",
            CursorMoved => "cursor-moved",
            ChatMessageSent => "chat-message",
            SessionTerminated => "session-terminated",
            AuthenticationSucceeded => "auth-ok",
            AuthenticationFailed => "auth-failed",
            UnauthorizedOperation => "unauthorized",
            PermissionChanged => "permission-changed",
            TerminalStarted => "terminal-started",
            TerminalInput => "terminal-input",
            TerminalOutput => "terminal-output",
            BuildStarted => "build-started",
            BuildFinished => "build-finished",
            SnapshotCreated => "snapshot-created",
            StorageWriteCompleted => "storage-write-completed",
            System => "system",
        }
    }
}

/// Typed payload carried by an event (spec section 13). Payloads map 1:1 to
/// an [`EventType`] via [`EventPayload::event_type`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventPayload {
    None,
    CubeCreated {
        name: String,
    },
    CubeDeleted {
        name: String,
    },
    FileCreated {
        path: String,
    },
    FileDeleted {
        path: String,
    },
    FileRenamed {
        from: String,
        to: String,
    },
    FileEdited {
        path: String,
        version: Version,
    },
    CursorMoved {
        path: String,
        line: u64,
        column: u64,
    },
    SelectionChanged {
        path: String,
        start: u64,
        end: u64,
    },
    UserJoined {
        user_id: UserId,
    },
    UserLeft {
        user_id: UserId,
    },
    ChatMessageSent {
        text: String,
    },
    SessionTerminated {
        reason: String,
    },
    AuthenticationSucceeded {},
    AuthenticationFailed {
        reason: String,
    },
    UnauthorizedOperation {
        operation: String,
        reason: String,
    },
    PermissionChanged {
        user_id: UserId,
    },
    TerminalStarted {
        command: String,
    },
    TerminalInput {
        data: Vec<u8>,
    },
    TerminalOutput {
        data: Vec<u8>,
    },
    BuildStarted {
        cube_id: CubeId,
    },
    BuildFinished {
        cube_id: CubeId,
        success: bool,
    },
    SnapshotCreated {
        snapshot_id: crate::ids::SnapshotId,
    },
    StorageWriteCompleted {
        path: String,
    },
}

impl EventPayload {
    /// Derives the event type implied by the payload.
    #[must_use]
    pub const fn event_type(&self) -> EventType {
        match self {
            EventPayload::None => EventType::System,
            EventPayload::CubeCreated { .. } => EventType::CubeCreated,
            EventPayload::CubeDeleted { .. } => EventType::CubeDeleted,
            EventPayload::FileCreated { .. } => EventType::FileCreated,
            EventPayload::FileDeleted { .. } => EventType::FileDeleted,
            EventPayload::FileRenamed { .. } => EventType::FileRenamed,
            EventPayload::FileEdited { .. } => EventType::FileEdited,
            EventPayload::CursorMoved { .. } => EventType::CursorMoved,
            EventPayload::SelectionChanged { .. } => EventType::SelectionChanged,
            EventPayload::UserJoined { .. } => EventType::UserJoined,
            EventPayload::UserLeft { .. } => EventType::UserLeft,
            EventPayload::ChatMessageSent { .. } => EventType::ChatMessageSent,
            EventPayload::SessionTerminated { .. } => EventType::SessionTerminated,
            EventPayload::AuthenticationSucceeded { .. } => EventType::AuthenticationSucceeded,
            EventPayload::AuthenticationFailed { .. } => EventType::AuthenticationFailed,
            EventPayload::UnauthorizedOperation { .. } => EventType::UnauthorizedOperation,
            EventPayload::PermissionChanged { .. } => EventType::PermissionChanged,
            EventPayload::TerminalStarted { .. } => EventType::TerminalStarted,
            EventPayload::TerminalInput { .. } => EventType::TerminalInput,
            EventPayload::TerminalOutput { .. } => EventType::TerminalOutput,
            EventPayload::BuildStarted { .. } => EventType::BuildStarted,
            EventPayload::BuildFinished { .. } => EventType::BuildFinished,
            EventPayload::SnapshotCreated { .. } => EventType::SnapshotCreated,
            EventPayload::StorageWriteCompleted { .. } => EventType::StorageWriteCompleted,
        }
    }

    /// The event category implied by this payload.
    #[must_use]
    pub const fn category(&self) -> EventCategory {
        self.event_type().category()
    }

    /// The delivery class implied by this payload.
    #[must_use]
    pub const fn delivery(&self) -> DeliveryClass {
        self.event_type().delivery()
    }
}

/// An immutable event record (spec section 5).
///
/// Every property is set at construction and never mutated afterward; the
/// "editor" helpers (`with_sequence`, `with_session`, ...) produce *new*
/// events instead of modifying this one. This immutability underpins replay,
/// audit, recovery, and synchronization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    id: EventId,
    event_type: EventType,
    category: EventCategory,
    timestamp: Timestamp,
    cube_id: CubeId,
    actor: UserId,
    session: Option<SessionId>,
    sequence: EventSequence,
    payload: EventPayload,
    schema_version: u32,
    delivery: DeliveryClass,
}

impl Event {
    /// Creates a new, un-sequenced event, deriving type, category, and
    /// delivery from the payload. The engine assigns a sequence before
    /// persisting/publishing.
    #[must_use]
    pub fn new(cube_id: CubeId, actor: UserId, payload: EventPayload) -> Self {
        let event_type = payload.event_type();
        Self {
            id: EventId::new(),
            event_type,
            category: event_type.category(),
            timestamp: Timestamp::now(),
            cube_id,
            actor,
            session: None,
            sequence: EventSequence::UNASSIGNED,
            payload,
            schema_version: EVENT_SCHEMA_VERSION,
            delivery: event_type.delivery(),
        }
    }

    /// Creates a new event at an explicit timestamp (used by tests/recovery).
    #[must_use]
    pub fn with_timestamp(
        cube_id: CubeId,
        actor: UserId,
        payload: EventPayload,
        timestamp: Timestamp,
    ) -> Self {
        let mut event = Self::new(cube_id, actor, payload);
        event.timestamp = timestamp;
        event
    }

    /// Produces a new event carrying the same data plus `sequence`. The
    /// original event is left untouched (immutability).
    #[must_use]
    pub fn with_sequence(&self, sequence: EventSequence) -> Self {
        let mut cloned = self.clone();
        cloned.sequence = sequence;
        cloned
    }

    /// Produces a new event stamped with the originating session.
    #[must_use]
    pub fn with_session(&self, session: Option<SessionId>) -> Self {
        let mut cloned = self.clone();
        cloned.session = session;
        cloned
    }

    #[must_use]
    pub const fn id(&self) -> EventId {
        self.id
    }

    #[must_use]
    pub const fn event_type(&self) -> EventType {
        self.event_type
    }

    #[must_use]
    pub const fn category(&self) -> EventCategory {
        self.category
    }

    #[must_use]
    pub const fn timestamp(&self) -> Timestamp {
        self.timestamp
    }

    #[must_use]
    pub const fn cube_id(&self) -> CubeId {
        self.cube_id
    }

    #[must_use]
    pub const fn actor(&self) -> UserId {
        self.actor
    }

    #[must_use]
    pub const fn session(&self) -> Option<SessionId> {
        self.session
    }

    #[must_use]
    pub const fn sequence(&self) -> EventSequence {
        self.sequence
    }

    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    #[must_use]
    pub const fn delivery(&self) -> DeliveryClass {
        self.delivery
    }

    #[must_use]
    pub const fn payload(&self) -> &EventPayload {
        &self.payload
    }

    /// Whether this event has been stamped by the sequencer yet.
    #[must_use]
    pub const fn is_sequenced(&self) -> bool {
        self.sequence.0 != EventSequence::UNASSIGNED.0
    }
}

impl std::fmt::Display for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} [seq {}] cube={} actor={} {}: {:?}",
            self.event_type.name(),
            self.sequence,
            self.cube_id,
            self.actor,
            self.category_str(),
            self.payload,
        )
    }
}

impl Event {
    #[must_use]
    pub const fn category_str(&self) -> &'static str {
        match self.category {
            EventCategory::Workspace => "workspace",
            EventCategory::Collaboration => "collaboration",
            EventCategory::Session => "session",
            EventCategory::Cube => "cube",
            EventCategory::Terminal => "terminal",
            EventCategory::Build => "build",
            EventCategory::Runtime => "runtime",
            EventCategory::Snapshot => "snapshot",
            EventCategory::Plugin => "plugin",
            EventCategory::Ai => "ai",
            EventCategory::System => "system",
            EventCategory::Security => "security",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_type_is_derived_from_payload() {
        let event = Event::new(
            CubeId::new(),
            UserId::new(),
            EventPayload::FileCreated {
                path: "/app/main.rs".into(),
            },
        );
        assert_eq!(event.event_type(), EventType::FileCreated);
        assert!(
            event.payload().clone()
                == EventPayload::FileCreated {
                    path: "/app/main.rs".into()
                }
        );
    }

    #[test]
    fn system_type_for_none_is_stable() {
        assert_eq!(EventPayload::None.event_type(), EventType::System);
    }

    #[test]
    fn category_and_delivery_are_derived() {
        let event = Event::new(
            CubeId::new(),
            UserId::new(),
            EventPayload::CursorMoved {
                path: "a.rs".into(),
                line: 3,
                column: 1,
            },
        );
        assert_eq!(event.category(), EventCategory::Workspace);
        assert_eq!(event.delivery(), DeliveryClass::Transient);
        assert!(!event.delivery().is_persistent());

        let durable = Event::new(
            CubeId::new(),
            UserId::new(),
            EventPayload::PermissionChanged {
                user_id: UserId::new(),
            },
        );
        assert_eq!(durable.category(), EventCategory::Security);
        assert_eq!(durable.delivery(), DeliveryClass::Durable);
        assert!(durable.delivery().is_persistent());
    }

    #[test]
    fn sequence_is_unassigned_until_stamped() {
        let event = Event::new(CubeId::new(), UserId::new(), EventPayload::None);
        assert_eq!(event.sequence(), EventSequence::UNASSIGNED);
        assert!(!event.is_sequenced());
        let stamped = event.with_sequence(EventSequence::new(7));
        assert_eq!(stamped.sequence().value(), 7);
        assert!(stamped.is_sequenced());
        // Immutability: the original is untouched.
        assert_eq!(event.sequence(), EventSequence::UNASSIGNED);
    }

    #[test]
    fn session_is_attachable_immutably() {
        let session = SessionId::new();
        let event = Event::new(CubeId::new(), UserId::new(), EventPayload::None);
        let stamped = event.with_session(Some(session));
        assert_eq!(stamped.session(), Some(session));
        assert_eq!(event.session(), None);
    }

    #[test]
    fn schema_version_is_current() {
        let event = Event::new(CubeId::new(), UserId::new(), EventPayload::None);
        assert_eq!(event.schema_version(), EVENT_SCHEMA_VERSION);
    }

    #[test]
    fn every_type_has_a_stable_name() {
        let types = [
            EventType::CubeCreated,
            EventType::AuthenticationFailed,
            EventType::CursorMoved,
            EventType::StorageWriteCompleted,
            EventType::System,
        ];
        for t in types {
            assert!(!t.name().is_empty());
            assert_eq!(t.category(), t.category());
        }
    }

    #[test]
    fn events_serialize_through_json() {
        let event = Event::new(
            CubeId::new(),
            UserId::new(),
            EventPayload::ChatMessageSent {
                text: "hello".into(),
            },
        )
        .with_sequence(EventSequence::new(3));
        let encoded = serde_json::to_string(&event).unwrap();
        let decoded: Event = serde_json::from_str(&encoded).unwrap();
        assert_eq!(event, decoded);
    }
}
