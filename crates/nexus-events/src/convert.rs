use nexus_core::event::{Event, EventCategory, EventType};
use nexus_core::ids::EventId;

/// Whether an event is intended for internal subsystems only or is safe to
/// surface to connected clients (spec section 54 framing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventVisibility {
    /// Only internal plumbing should see this (e.g. StorageWriteCompleted).
    Internal,
    /// Clients may see this.
    Client,
}

/// Distinguishes internal from client-visible events.
///
/// Internal plumbing events (storage confirms, system heartbeats) never leak
/// over the wire; everything else derives its visibility from the registry
/// (which the conversion layer consults before serializing).
#[must_use]
pub fn visibility(event: &Event) -> EventVisibility {
    let internal = matches!(
        event.event_type(),
        EventType::StorageWriteCompleted | EventType::System
    );
    if internal {
        EventVisibility::Internal
    } else {
        EventVisibility::Client
    }
}

/// Which categories are considered "user-intent" events for collaboration
/// surfaces — everything except pure system plumbing.
#[must_use]
pub fn is_collaboration_relevant(event: &Event) -> bool {
    matches!(
        event.category(),
        EventCategory::Collaboration
            | EventCategory::Workspace
            | EventCategory::Terminal
            | EventCategory::Build
            | EventCategory::Cube
    )
}

/// A redacted, client-safe view of an event (spec section 54 ensures internal
/// payload details are never serialized to clients).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClientEventView {
    pub id: EventId,
    pub event_type: String,
    pub category: String,
    pub sequence: u64,
    pub cube: String,
    pub actor: String,
    pub timestamp_ms: u64,
}

impl ClientEventView {
    /// Builds a client view, or `None` for internal-only events.
    #[must_use]
    pub fn from_event(event: &Event) -> Option<Self> {
        if visibility(event) == EventVisibility::Internal {
            return None;
        }
        Some(Self {
            id: event.id(),
            event_type: event.event_type().name().to_string(),
            category: event.category_str().to_string(),
            sequence: event.sequence().value(),
            cube: event.cube_id().to_string(),
            actor: event.actor().to_string(),
            timestamp_ms: event.timestamp().as_millis(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::event::{Event, EventPayload};
    use nexus_core::ids::{CubeId, UserId};

    fn ev(payload: EventPayload) -> Event {
        Event::new(CubeId::new(), UserId::new(), payload)
            .with_sequence(nexus_core::event::EventSequence::new(1))
    }

    #[test]
    fn internal_events_are_hidden_from_clients() {
        let hidden = ev(EventPayload::StorageWriteCompleted {
            path: "/secret".into(),
        });
        assert_eq!(visibility(&hidden), EventVisibility::Internal);
        assert!(ClientEventView::from_event(&hidden).is_none());

        let visible = ev(EventPayload::FileEdited {
            path: "a.rs".into(),
            version: nexus_core::version::Version::new(1),
        });
        assert_eq!(visibility(&visible), EventVisibility::Client);
        let view = ClientEventView::from_event(&visible).unwrap();
        assert_eq!(view.event_type, "file-edited");
        assert!(debug_roundtrips_json(&view));
    }

    fn debug_roundtrips_json(view: &ClientEventView) -> bool {
        let encoded = serde_json::to_string(view).ok();
        match encoded {
            Some(text) => serde_json::from_str::<ClientEventView>(&text).ok() == Some(view.clone()),
            None => false,
        }
    }

    #[test]
    fn collaboration_windows_are_stable() {
        assert!(is_collaboration_relevant(&ev(EventPayload::CursorMoved {
            path: "a".into(),
            line: 0,
            column: 0
        })));
        assert!(!is_collaboration_relevant(&ev(EventPayload::None)));
    }
}
