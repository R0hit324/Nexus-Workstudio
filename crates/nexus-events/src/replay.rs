use nexus_core::event::EventSequence;
use nexus_core::ids::CubeId;

use crate::store::EventStore;
use crate::Result;

/// A resumable view over a Cube's event log (spec section 42, 48).
///
/// Replay reads events in strict sequence order and yields them grouped;
/// the caller drives a consumer/projection and can persist a checkpoint to
/// resume exactly where it stopped.
#[derive(Debug)]
pub struct ReplayCursor<'a> {
    store: &'a dyn EventStore,
    cube: CubeId,
    next: EventSequence,
}

impl<'a> ReplayCursor<'a> {
    /// Starts replay at `from` (inclusive) for `cube`.
    pub fn at(store: &'a dyn EventStore, cube: CubeId, from: EventSequence) -> Self {
        Self {
            store,
            cube,
            next: from,
        }
    }

    /// The next sequence the cursor will yield.
    #[must_use]
    pub const fn next_sequence(&self) -> EventSequence {
        self.next
    }

    /// Fetches up to `limit` events, returning how many were read.
    pub fn next_n(&mut self, limit: usize) -> Result<Vec<nexus_core::event::Event>> {
        let mut out = Vec::with_capacity(limit);
        let all = self.store.replay(self.cube, self.next)?;
        for event in all.into_iter().take(limit) {
            let seq = event.sequence();
            out.push(event);
            self.next = seq.next();
        }
        Ok(out)
    }

    /// Advances the cursor past everything already stored for the Cube.
    pub fn fast_forward(&mut self) -> Result<EventSequence> {
        let latest = self.store.latest_sequence(self.cube);
        self.next = latest.next();
        Ok(latest)
    }
}

/// Convenience: replays the full log of `cube` from a sequence (inclusive)
/// and returns every event in order.
pub fn replay_from(
    store: &dyn EventStore,
    cube: CubeId,
    from: EventSequence,
) -> Result<Vec<nexus_core::event::Event>> {
    store.replay(cube, from)
}

/// Replays a log into any consumer, applying each event and catching
/// consumer failures (spec section 49 "reconnection / resync").
pub fn replay_into<F>(
    store: &dyn EventStore,
    cube: CubeId,
    from: EventSequence,
    mut apply: F,
) -> Result<u64>
where
    F: FnMut(&nexus_core::event::Event) -> crate::Result<()>,
{
    let events = store.replay(cube, from)?;
    let mut applied = 0u64;
    for event in &events {
        apply(event)?;
        applied += 1;
    }
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryEventStore;
    use nexus_core::event::{Event, EventPayload, EventType};
    use nexus_core::ids::UserId;

    fn ev(cube: CubeId, seq: u64) -> Event {
        Event::new(
            cube,
            UserId::new(),
            EventPayload::FileCreated {
                path: format!("/f{seq}"),
            },
        )
        .with_sequence(EventSequence::new(seq))
    }

    #[test]
    fn cursor_streams_in_chunks() {
        let store = MemoryEventStore::new();
        let cube = CubeId::new();
        for seq in 1..=10 {
            store.append(&ev(cube, seq)).unwrap();
        }
        let mut cursor = ReplayCursor::at(&store, cube, EventSequence::new(3));
        let chunk_a = cursor.next_n(4).unwrap();
        assert_eq!(chunk_a.len(), 4);
        assert_eq!(chunk_a[0].sequence().value(), 3);
        assert_eq!(cursor.next_sequence().value(), 7);
        let chunk_b = cursor.next_n(100).unwrap();
        assert_eq!(chunk_b.len(), 4);
        assert_eq!(cursor.next_sequence().value(), 11);
    }

    #[test]
    fn fast_forward_skips_to_end() {
        let store = MemoryEventStore::new();
        let cube = CubeId::new();
        for seq in 1..=4 {
            store.append(&ev(cube, seq)).unwrap();
        }
        let mut cursor = ReplayCursor::at(&store, cube, EventSequence::new(1));
        let latest = cursor.fast_forward().unwrap();
        assert_eq!(latest.value(), 4);
        assert_eq!(cursor.next_sequence().value(), 5);
    }

    #[test]
    fn replay_into_fans_out() {
        let store = MemoryEventStore::new();
        let cube = CubeId::new();
        for seq in 1..=3 {
            store.append(&ev(cube, seq)).unwrap();
        }
        let mut seen = Vec::new();
        let applied = replay_into(&store, cube, EventSequence::new(1), |e| {
            assert_eq!(e.event_type(), EventType::FileCreated);
            seen.push(e.sequence().value());
            Ok(())
        })
        .unwrap();
        assert_eq!(applied, 3);
        assert_eq!(seen, vec![1, 2, 3]);
    }
}
