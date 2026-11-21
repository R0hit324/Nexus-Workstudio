use std::collections::{HashSet, VecDeque};

use nexus_core::ids::EventId;

/// Detects duplicate redeliveries of events by [`EventId`].
///
/// Delivered events are remembered for a bounded window (spec section 40). A
/// consumer that replays a stream uses this to skip duplicates that arrive
/// over the wire or from a redundant replay.
#[derive(Debug)]
pub struct AckTracker {
    window: usize,
    seen: HashSet<EventId>,
    order: VecDeque<EventId>,
    pub accepted: u64,
    pub duplicates: u64,
}

impl AckTracker {
    /// Remembers the `window` most recently seen ids.
    #[must_use]
    pub fn with_window(window: usize) -> Self {
        Self {
            window,
            seen: HashSet::new(),
            order: VecDeque::new(),
            accepted: 0,
            duplicates: 0,
        }
    }

    /// Records an id if it hasn't been seen within the window.
    /// Returns `false` (and counts a duplicate) when it already was.
    pub fn accept(&mut self, id: EventId) -> bool {
        if self.seen.contains(&id) {
            self.duplicates += 1;
            return false;
        }
        self.accepted += 1;
        self.seen.insert(id);
        self.order.push_back(id);
        while self.order.len() > self.window {
            if let Some(evicted) = self.order.pop_front() {
                self.seen.remove(&evicted);
            }
        }
        true
    }

    /// Whether `id` was seen within the window.
    #[must_use]
    pub fn contains(&self, id: EventId) -> bool {
        self.seen.contains(&id)
    }
}

impl Default for AckTracker {
    fn default() -> Self {
        Self::with_window(8192)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_is_accepted_second_is_duplicate() {
        let mut tracker = AckTracker::with_window(16);
        let id = EventId::new();
        assert!(tracker.accept(id));
        assert!(!tracker.accept(id));
        assert_eq!(tracker.accepted, 1);
        assert_eq!(tracker.duplicates, 1);
    }

    #[test]
    fn window_evicts_old_ids() {
        let mut tracker = AckTracker::with_window(3);
        let ids: Vec<_> = (0..4).map(|_| EventId::new()).collect();
        for id in &ids {
            tracker.accept(*id);
        }
        // First id was evicted, so it is accepted (and counted) once more.
        assert!(tracker.accept(ids[0]));
        assert!(!tracker.accept(ids[3]));
    }
}
