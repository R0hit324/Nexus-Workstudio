use std::collections::HashMap;
use std::sync::Mutex;

use nexus_core::event::Event;
use nexus_core::ids::CubeId;
use tokio::sync::broadcast;

/// Size of each per-Cube broadcast channel. Messages are dropped (not queued
/// forever) when the subscriber is too slow.
const PER_CUBE_CHANNEL_CAPACITY: usize = 256;

/// A simple, cube-scoped event backbone.
///
/// Part 3 ships the skeleton; Part 4 fills in full fan-out via the connection
/// layer. The [`tokio::sync::broadcast`] channel provides backpressure-free
/// fan-out: slow subscribers miss messages rather than block the producer.
pub struct EventBus {
    channels: Mutex<HashMap<CubeId, broadcast::Sender<Event>>>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            channels: Mutex::new(HashMap::new()),
        }
    }

    /// Subscribes to every event in `cube_id`. Returns a new receiver; each
    /// call is independent.
    pub fn subscribe(&self, cube_id: CubeId) -> broadcast::Receiver<Event> {
        self.channel_for(cube_id).subscribe()
    }

    /// Pushes `event` into the `cube_id` channel. Returns the number of live
    /// receivers at the time of the call (0 when nobody is listening).
    pub fn publish(&self, cube_id: CubeId, event: Event) -> usize {
        let sender = self.channel_for(cube_id);
        let receivers = sender.receiver_count();
        let _ = sender.send(event);
        receivers
    }

    /// Removes the cube's channel once no sessions are attached to it. The
    /// next subscriber re-creates a fresh channel automatically.
    pub fn detach_cube(&self, cube_id: CubeId) {
        self.channels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&cube_id);
    }

    fn channel_for(&self, cube_id: CubeId) -> broadcast::Sender<Event> {
        self.channels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(cube_id)
            .or_insert_with(|| broadcast::channel(PER_CUBE_CHANNEL_CAPACITY).0)
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::event::EventPayload;

    fn presence(cube_id: CubeId) -> Event {
        Event::new(
            cube_id,
            nexus_core::ids::UserId::new(),
            EventPayload::UserJoined {
                user_id: nexus_core::ids::UserId::new(),
            },
        )
    }

    #[test]
    fn publish_delivers_to_subscribers() {
        let bus = EventBus::new();
        let cube = CubeId::new();
        let mut rx1 = bus.subscribe(cube);
        let mut rx2 = bus.subscribe(cube);
        bus.publish(cube, presence(cube));
        assert!(matches!(rx1.try_recv(), Ok(_)));
        assert!(matches!(rx2.try_recv(), Ok(_)));
    }

    #[test]
    fn publish_to_unknown_cube_returns_zero() {
        let bus = EventBus::new();
        let cube = CubeId::new();
        assert_eq!(bus.publish(cube, presence(cube)), 0);
    }

    #[test]
    fn detach_removes_channel() {
        let bus = EventBus::new();
        let cube = CubeId::new();
        let _ = bus.subscribe(cube);
        bus.detach_cube(cube);
        assert!(bus
            .channels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_empty());
    }
}
