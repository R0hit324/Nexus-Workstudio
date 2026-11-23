use nexus_core::event::Event;
use nexus_core::ids::RequestId;

/// Structured, low-overhead tracing for events (spec section 56).
///
/// The helper emits a single `debug` event with the event's identity so
/// operational tools can correlate an event across sequence assignment,
/// persistence, fan-out, and consumer application via the same span context.
pub fn trace_event(event: &Event, request_id: Option<&RequestId>) {
    tracing::debug!(
        event.id = %event.id(),
        event.type = event.event_type().name(),
        event.seq = %event.sequence(),
        event.category = event.category_str(),
        event.cube = %event.cube_id(),
        event.actor = %event.actor(),
        event.session = ?event.session().map(|s| s.to_string()),
        event.request = ?request_id.map(|r| r.to_string()),
        "event lifecycle"
    );
}

/// Formats a compact single-line log-friendly summary of an event (used when
/// `tracing` is disabled or for tests).
#[must_use]
pub fn format_event(event: &Event) -> String {
    format!(
        "{} seq={} cube={} actor={} {}, {}ms",
        event.event_type().name(),
        event.sequence(),
        event.cube_id(),
        event.actor(),
        event.category_str(),
        event.timestamp().as_millis(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::event::{Event, EventPayload};
    use nexus_core::ids::{CubeId, UserId};

    #[test]
    fn trace_event_produces_stable_summary() {
        let event = Event::new(
            CubeId::new(),
            UserId::new(),
            EventPayload::ChatMessageSent { text: "hi".into() },
        )
        .with_sequence(nexus_core::event::EventSequence::new(1));
        let summary = format_event(&event);
        assert!(summary.contains("chat-message"));
        assert!(summary.contains("seq=1"));

        trace_event(&event, None);
        trace_event(&event, Some(&RequestId::new()));
    }
}
