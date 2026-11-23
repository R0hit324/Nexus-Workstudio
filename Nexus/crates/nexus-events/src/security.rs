use nexus_core::event::{Event, EventPayload, EventType};
use nexus_core::ids::{CubeId, SessionId, UserId};

/// Constructors for security-related events (spec section 54).
///
/// Security events are always Durable: they must survive restart and be
/// available for audit/forensics. The registry validates this invariant so a
/// mis-registered type cannot silently downgrade an audit record.
pub struct SecurityEvent;

impl SecurityEvent {
    /// Emitted after a login attempt succeeds.
    #[must_use]
    pub fn authentication_succeeded(cube: CubeId, actor: UserId, session: SessionId) -> Event {
        Event::new(cube, actor, EventPayload::AuthenticationSucceeded {})
            .with_session(Some(session))
    }

    /// Emitted after a login attempt fails.
    #[must_use]
    pub fn authentication_failed(cube: CubeId, actor: UserId, reason: impl Into<String>) -> Event {
        Event::new(
            cube,
            actor,
            EventPayload::AuthenticationFailed {
                reason: reason.into(),
            },
        )
    }

    /// Emitted when a principal attempts something they don't have permission
    /// for. Recorded as `UnauthorizedOperation` with the operation name.
    #[must_use]
    pub fn unauthorized(
        cube: CubeId,
        actor: UserId,
        operation: impl Into<String>,
        reason: impl Into<String>,
    ) -> Event {
        Event::new(
            cube,
            actor,
            EventPayload::UnauthorizedOperation {
                operation: operation.into(),
                reason: reason.into(),
            },
        )
    }

    /// Emitted when a user's permissions change (e.g. read-only re-grant).
    #[must_use]
    pub fn permission_changed(
        cube: CubeId,
        actor: UserId,
        subject: UserId,
        session: Option<SessionId>,
    ) -> Event {
        Event::new(
            cube,
            actor,
            EventPayload::PermissionChanged { user_id: subject },
        )
        .with_session(session)
    }

    /// Emitted when a session is terminated (explicitly or by timeout).
    #[must_use]
    pub fn session_terminated(cube: CubeId, actor: UserId, reason: impl Into<String>) -> Event {
        Event::new(
            cube,
            actor,
            EventPayload::SessionTerminated {
                reason: reason.into(),
            },
        )
    }

    /// Whether an event belongs to the security category.
    #[must_use]
    pub fn is_security(event: &Event) -> bool {
        matches!(
            event.event_type(),
            EventType::AuthenticationSucceeded
                | EventType::AuthenticationFailed
                | EventType::UnauthorizedOperation
                | EventType::PermissionChanged
                | EventType::SessionTerminated
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::event::{DeliveryClass, EventCategory};

    #[test]
    fn security_events_are_durable_and_auditable() {
        let cube = CubeId::new();
        let actor = UserId::new();
        let events = [
            SecurityEvent::authentication_succeeded(cube, actor, SessionId::new()),
            SecurityEvent::authentication_failed(cube, actor, "bad password"),
            SecurityEvent::unauthorized(cube, actor, "read-dir", "no permission"),
            SecurityEvent::permission_changed(cube, actor, actor, None),
            SecurityEvent::session_terminated(cube, actor, "idle"),
        ];
        for event in &events {
            assert!(
                SecurityEvent::is_security(event),
                "{event:?} flagged security"
            );
            assert_eq!(event.delivery(), DeliveryClass::Durable);
        }
        // Category is "security" for the four pure security types; session
        // termination is durable/auditable but grouped under Session.
        for event in &events[..4] {
            assert_eq!(event.category(), EventCategory::Security);
        }
        assert_eq!(events[4].category(), EventCategory::Session);
    }
}
