//! Outbound event queue consumed by the application loop.

use std::collections::VecDeque;

use super::network::NetworkEvent;

/// A FIFO of network events between the transport and the UI/headless caller.
#[derive(Debug, Default)]
pub struct EventSink {
    pending: VecDeque<NetworkEvent>,
}

impl EventSink {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues an event.
    pub fn push(&mut self, event: NetworkEvent) {
        self.pending.push_back(event);
    }

    /// Drains all pending events in order, updating `ClientState` via
    /// [`crate::app::state::ClientState::drain_events`].
    #[must_use]
    pub fn drain(&mut self, state: &mut crate::app::state::ClientState) -> Vec<NetworkEvent> {
        let events: Vec<NetworkEvent> = self.pending.drain(..).collect();
        state.drain_events(events.clone());
        events
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.pending.len()
    }
}
