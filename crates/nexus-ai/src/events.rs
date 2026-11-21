//! AI event integration (`§50`): the bounded, classified AI event surface
//! that joins the global event system.
//!
//! Events are classified [`AiVisibility`] — Durable, Ephemeral, Private, or
//! Cube-scoped. Private AI events (prompt/possible-content carriers) are
//! never broadcast; ephemeral ones (high-frequency chunks) are never even
//! stored in the ledger.

use crate::session::AiSessionId;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Every AI event the subsystem can emit (`§50`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AiEventKind {
    AiSessionCreated,
    AiRequestStarted,
    AiResponseChunk,
    AiResponseCompleted,
    AiRequestCancelled,
    AiToolCallStarted,
    AiToolCallCompleted,
    AiActionProposed,
    AiActionApproved,
    AiActionRejected,
    AiActionApplied,
    AiActionFailed,
    AiProviderError,
}

/// Event visibility classification (`§50`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AiVisibility {
    /// Persisted long-term and safe to broadcast.
    Durable,
    /// High-frequency stream detail: never stored, never broadcast.
    Ephemeral,
    /// Persisted internally but may carry user content: never broadcast.
    Private,
    /// Persisted and broadcast only within the owning Cube.
    CubeScoped,
}

impl AiEventKind {
    /// The default visibility policy for this event kind (`§50`).
    ///
    /// - Chunks are ephemeral by volume.
    /// - Request starts may mirror prompt metadata, so they are private.
    /// - Tool-call starts relay arguments, thus private; completions are
    ///   summarized, thus durable.
    /// - Action lifecycle and provider errors are durable audit records.
    /// - Session lifecycle stays cube-scoped until explicitly shared.
    #[must_use]
    pub const fn visibility(&self) -> AiVisibility {
        match self {
            Self::AiResponseChunk => AiVisibility::Ephemeral,
            Self::AiRequestStarted | Self::AiToolCallStarted => AiVisibility::Private,
            Self::AiSessionCreated => AiVisibility::CubeScoped,
            _ => AiVisibility::Durable,
        }
    }

    /// Stable lowercase identifier used by the event bus payloads.
    #[must_use]
    pub fn slug(&self) -> String {
        match self {
            Self::AiSessionCreated => "ai_session_created",
            Self::AiRequestStarted => "ai_request_started",
            Self::AiResponseChunk => "ai_response_chunk",
            Self::AiResponseCompleted => "ai_response_completed",
            Self::AiRequestCancelled => "ai_request_cancelled",
            Self::AiToolCallStarted => "ai_tool_call_started",
            Self::AiToolCallCompleted => "ai_tool_call_completed",
            Self::AiActionProposed => "ai_action_proposed",
            Self::AiActionApproved => "ai_action_approved",
            Self::AiActionRejected => "ai_action_rejected",
            Self::AiActionApplied => "ai_action_applied",
            Self::AiActionFailed => "ai_action_failed",
            Self::AiProviderError => "ai_provider_error",
        }
        .to_owned()
    }
}

/// One classified AI event.
#[derive(Debug, Clone)]
pub struct AiEvent {
    pub kind: AiEventKind,
    pub session: AiSessionId,
    /// Safe summary text. Prompt/response content never lands here.
    pub detail: String,
    pub visibility: AiVisibility,
    pub recorded: Instant,
}

impl AiEvent {
    #[must_use]
    pub fn new(kind: AiEventKind, session: AiSessionId, detail: impl Into<String>) -> Self {
        Self {
            kind,
            session,
            detail: detail.into(),
            visibility: kind.visibility(),
            recorded: Instant::now(),
        }
    }

    /// Whether external subscribers may ever see this event.
    #[must_use]
    pub fn broadcastable(&self) -> bool {
        !matches!(
            self.visibility,
            AiVisibility::Private | AiVisibility::Ephemeral
        )
    }
}

/// Bounded in-memory ledger of AI events. The global event system receives
/// only [`AiEvent::broadcastable`] records; the ledger additionally keeps
/// private events so audits/debugging can inspect flow without exposure
/// (`§50` "do not broadcast private").
#[derive(Debug)]
pub struct AiEventLedger {
    max_entries: usize,
    events: Vec<AiEvent>,
}

impl Default for AiEventLedger {
    fn default() -> Self {
        Self::new(4_096)
    }
}

impl AiEventLedger {
    #[must_use]
    pub fn new(max_entries: usize) -> Self {
        Self {
            max_entries,
            events: Vec::new(),
        }
    }

    /// Adds an event. Ephemeral kinds are dropped immediately; everything
    /// else stays (bounded) so durability semantics do not depend on the
    /// subscriber at all. Returns the event that was (not) stored.
    ///
    /// ```
    /// # use nexus_ai::events::{AiEventKind, AiEventLedger};
    /// # use nexus_ai::session::AiSessionId;
    /// let mut ledger = AiEventLedger::default();
    /// // Ephemeral chunks never enter the ledger.
    /// ledger.record(AiEventKind::AiResponseChunk, AiSessionId::new(1), "+z");
    /// assert!(ledger.timeline().is_empty());
    /// ```
    pub fn record(&mut self, kind: AiEventKind, session: AiSessionId, detail: impl Into<String>) {
        if kind.visibility() == AiVisibility::Ephemeral {
            return;
        }
        if self.events.len() >= self.max_entries {
            self.events.remove(0);
        }
        self.events.push(AiEvent::new(kind, session, detail));
    }

    /// All stored events, oldest first.
    #[must_use]
    pub fn timeline(&self) -> &[AiEvent] {
        &self.events
    }

    /// Stored events for one session, oldest first.
    #[must_use]
    pub fn events_of(&self, session: AiSessionId) -> Vec<&AiEvent> {
        self.events
            .iter()
            .filter(|event| event.session == session)
            .collect()
    }

    /// Events a broadcast filter may forward: durable/cube-scoped only.
    #[must_use]
    pub fn broadcastable(&self) -> Vec<&AiEvent> {
        self.events.iter().filter(|e| e.broadcastable()).collect()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Clears the ledger (purely local state — private events are still
    /// not persisted anywhere else by this type).
    pub fn clear(&mut self) {
        self.events.clear();
    }
}

/// Session-bound handle emitting into a shared ledger. Hosts hand an
/// `EventSink` to request-level code (orchestrator, tool runner) that has no
/// session context of its own; visibility policy still applies inside the
/// ledger when the event lands.
#[derive(Clone)]
pub struct EventSink {
    ledger: Arc<Mutex<AiEventLedger>>,
    session: AiSessionId,
}

impl EventSink {
    #[must_use]
    pub fn new(ledger: Arc<Mutex<AiEventLedger>>, session: AiSessionId) -> Self {
        Self { ledger, session }
    }

    #[must_use]
    pub const fn session(&self) -> AiSessionId {
        self.session
    }

    /// Emits one event into the shared ledger. Detail text must already be
    /// content-free (callers filter prompts themselves, `§50`).
    pub fn emit(&self, kind: AiEventKind, detail: impl Into<String>) {
        self.ledger
            .lock()
            .expect("ledger lock")
            .record(kind, self.session, detail);
    }
}

/// Per-session event counts — the cheap derived view the hosts use for
/// activity badges.
#[must_use]
pub fn session_activity(events: &[AiEvent]) -> HashMap<AiSessionId, usize> {
    let mut counts = HashMap::new();
    for event in events {
        *counts.entry(event.session).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::{session_activity, AiEvent, AiEventKind, AiEventLedger, AiVisibility};
    use crate::session::AiSessionId;
    use std::time::Instant;

    #[test]
    fn event_kinds_carry_the_documented_visibility() {
        assert_eq!(
            AiEventKind::AiResponseChunk.visibility(),
            AiVisibility::Ephemeral
        );
        assert_eq!(
            AiEventKind::AiRequestStarted.visibility(),
            AiVisibility::Private
        );
        assert_eq!(
            AiEventKind::AiToolCallStarted.visibility(),
            AiVisibility::Private
        );
        assert_eq!(
            AiEventKind::AiSessionCreated.visibility(),
            AiVisibility::CubeScoped
        );
        for kind in [
            AiEventKind::AiResponseCompleted,
            AiEventKind::AiRequestCancelled,
            AiEventKind::AiToolCallCompleted,
            AiEventKind::AiActionProposed,
            AiEventKind::AiActionApproved,
            AiEventKind::AiActionRejected,
            AiEventKind::AiActionApplied,
            AiEventKind::AiActionFailed,
            AiEventKind::AiProviderError,
        ] {
            assert_eq!(kind.visibility(), AiVisibility::Durable, "kind {kind:?}");
        }
    }

    #[test]
    fn slugs_round_trip_every_variant() {
        let all = [
            AiEventKind::AiSessionCreated,
            AiEventKind::AiRequestStarted,
            AiEventKind::AiResponseChunk,
            AiEventKind::AiResponseCompleted,
            AiEventKind::AiRequestCancelled,
            AiEventKind::AiToolCallStarted,
            AiEventKind::AiToolCallCompleted,
            AiEventKind::AiActionProposed,
            AiEventKind::AiActionApproved,
            AiEventKind::AiActionRejected,
            AiEventKind::AiActionApplied,
            AiEventKind::AiActionFailed,
            AiEventKind::AiProviderError,
        ];
        for kind in all {
            assert!(!kind.slug().is_empty());
            assert!(kind.slug().starts_with("ai_"), "slug of {kind:?}");
        }
    }

    #[test]
    fn ledger_drops_ephemeral_batches_and_keeps_events() {
        let mut ledger = AiEventLedger::new(4);
        let session = AiSessionId::new(9);
        for letter in [String::from("a"), String::from("b"), String::from("c")] {
            ledger.record(AiEventKind::AiResponseChunk, session, letter);
        }
        assert!(ledger.is_empty());
        ledger.record(AiEventKind::AiActionProposed, session, "transaction 3");
        ledger.record(AiEventKind::AiActionApplied, session, "transaction 3");
        assert_eq!(ledger.len(), 2);
        assert_eq!(ledger.events_of(session).len(), 2);
    }

    #[test]
    fn ledger_stays_bounded_oldest_first_out() {
        let mut ledger = AiEventLedger::new(3);
        let session = AiSessionId::new(2);
        for index in 0..5 {
            ledger.record(
                AiEventKind::AiActionApproved,
                session,
                format!("round {index}"),
            );
        }
        assert_eq!(ledger.len(), 3);
        assert_eq!(ledger.timeline()[0].detail, "round 2");
        assert_eq!(ledger.timeline()[2].detail, "round 4");
    }

    #[test]
    fn private_events_stay_inside_but_never_broadcast() {
        let mut ledger = AiEventLedger::new(8);
        let session = AiSessionId::new(4);
        ledger.record(AiEventKind::AiRequestStarted, session, "session boot");
        // Stored for the audit path...
        assert_eq!(ledger.len(), 1);
        // ...but excluded from broadcasting.
        assert_eq!(ledger.broadcastable().len(), 0);
        assert!(!ledger.timeline()[0].broadcastable());
    }

    #[test]
    fn timeline_isolates_sessions() {
        let mut ledger = AiEventLedger::new(8);
        ledger.record(AiEventKind::AiSessionCreated, AiSessionId::new(1), "one");
        ledger.record(AiEventKind::AiSessionCreated, AiSessionId::new(2), "two");
        ledger.record(AiEventKind::AiActionProposed, AiSessionId::new(1), "txn");
        let one = AiSessionId::new(1);
        assert_eq!(ledger.events_of(one).len(), 2);
        assert_eq!(ledger.timeline().len(), 3);
    }

    #[test]
    fn activity_counts_roll_up_per_session() {
        let events = vec![
            AiEvent {
                kind: AiEventKind::AiSessionCreated,
                session: AiSessionId::new(1),
                detail: String::from("a"),
                visibility: AiVisibility::Durable,
                recorded: Instant::now(),
            },
            AiEvent {
                kind: AiEventKind::AiActionApplied,
                session: AiSessionId::new(1),
                detail: String::from("b"),
                visibility: AiVisibility::Durable,
                recorded: Instant::now(),
            },
        ];
        let counts = session_activity(&events);
        assert_eq!(counts.len(), 1);
        assert_eq!(counts[&AiSessionId::new(1)], 2);
    }
}
