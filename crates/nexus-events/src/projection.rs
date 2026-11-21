use std::collections::{BTreeMap, HashSet};

use nexus_core::event::{Event, EventSequence, EventType};
use nexus_core::ids::{CubeId, UserId};
use nexus_core::timestamp::Timestamp;

use crate::consumer::Consumer;
use crate::Result;

/// The projection pattern (spec section 43): a derived value built from the
/// event log. Projectors are [`Consumer`]s whose state is queryable; they
/// rebuild by replaying the log from a checkpoint.
pub trait Projector: Consumer {}

/// Live members per Cube, built from `UserJoined`/`UserLeft` events.
#[derive(Debug, Default)]
pub struct PresenceProjection {
    members: BTreeMap<CubeId, HashSet<UserId>>,
}

impl PresenceProjection {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Members currently present in `cube` (join minus leave, deduped).
    #[must_use]
    pub fn members(&self, cube: CubeId) -> Vec<UserId> {
        let mut v: Vec<UserId> = self
            .members
            .get(&cube)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default();
        v.sort();
        v
    }

    /// Whether `user` is present in `cube`.
    #[must_use]
    pub fn is_present(&self, cube: CubeId, user: UserId) -> bool {
        self.members
            .get(&cube)
            .map(|s| s.contains(&user))
            .unwrap_or(false)
    }
}

impl Consumer for PresenceProjection {
    fn name(&self) -> &'static str {
        "presence"
    }

    fn handle(&mut self, event: &Event) -> Result<()> {
        if event.category() == nexus_core::event::EventCategory::Collaboration {
            match event.payload() {
                nexus_core::event::EventPayload::UserJoined { user_id } => {
                    self.members
                        .entry(event.cube_id())
                        .or_default()
                        .insert(*user_id);
                }
                nexus_core::event::EventPayload::UserLeft { user_id } => {
                    if let Some(set) = self.members.get_mut(&event.cube_id()) {
                        set.remove(user_id);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

impl Projector for PresenceProjection {}

/// File-tree metadata counts per workspace, from create/delete/rename events.
#[derive(Debug, Default)]
pub struct WorkspaceMetadataProjection {
    files: BTreeMap<CubeId, HashSet<String>>,
}

impl WorkspaceMetadataProjection {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// All recorded file paths for `cube`.
    #[must_use]
    pub fn files(&self, cube: CubeId) -> Vec<String> {
        self.files
            .get(&cube)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    #[must_use]
    pub fn file_count(&self, cube: CubeId) -> usize {
        self.files.get(&cube).map(HashSet::len).unwrap_or(0)
    }
}

impl Consumer for WorkspaceMetadataProjection {
    fn name(&self) -> &'static str {
        "workspace-metadata"
    }

    fn handle(&mut self, event: &Event) -> Result<()> {
        let files = self.files.entry(event.cube_id()).or_default();
        match event.payload() {
            nexus_core::event::EventPayload::FileCreated { path } => {
                files.insert(path.clone());
            }
            nexus_core::event::EventPayload::FileDeleted { path } => {
                files.remove(path);
            }
            nexus_core::event::EventPayload::FileRenamed { from, to } => {
                files.remove(from);
                files.insert(to.clone());
            }
            _ => {}
        }
        Ok(())
    }
}

impl Projector for WorkspaceMetadataProjection {}

/// A bounded, most-recent activity feed, fed by all client-visible events.
#[derive(Debug)]
pub struct ActivityFeedProjection {
    capacity: usize,
    entries: BTreeMap<EventSequence, ActivityEntry>,
    counter: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityEntry {
    pub sequence: EventSequence,
    pub event_type: EventType,
    pub actor: UserId,
    pub cube: CubeId,
    pub timestamp: Timestamp,
}

impl ActivityFeedProjection {
    /// A feed keeping the `capacity` most recent entries per Cube.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: BTreeMap::new(),
            counter: 0,
        }
    }

    /// Recent entries across all Cubes, newest first.
    #[must_use]
    pub fn recent(&self) -> Vec<ActivityEntry> {
        self.entries
            .values()
            .rev()
            .take(self.capacity)
            .cloned()
            .collect()
    }

    /// Recent entries for one Cube.
    #[must_use]
    pub fn recent_in(&self, cube: CubeId) -> Vec<ActivityEntry> {
        self.entries
            .values()
            .rev()
            .filter(|e| e.cube == cube)
            .take(self.capacity)
            .cloned()
            .collect()
    }
}

impl Consumer for ActivityFeedProjection {
    fn name(&self) -> &'static str {
        "activity-feed"
    }

    fn handle(&mut self, event: &Event) -> Result<()> {
        if !crate::convert::is_collaboration_relevant(event) {
            return Ok(());
        }
        self.counter += 1;
        let seq = event.sequence();
        self.entries.insert(
            seq,
            ActivityEntry {
                sequence: seq,
                event_type: event.event_type(),
                actor: event.actor(),
                cube: event.cube_id(),
                timestamp: event.timestamp(),
            },
        );
        while self.entries.len() > self.capacity {
            // Evict the oldest sequence values.
            if let Some((&oldest, _)) = self.entries.iter().next() {
                self.entries.remove(&oldest);
            }
        }
        Ok(())
    }
}

impl Projector for ActivityFeedProjection {}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::event::{Event, EventPayload};
    use nexus_core::ids::CubeId;

    fn join(cube: CubeId, user: UserId, seq: u64) -> Event {
        Event::new(
            cube,
            UserId::new(),
            EventPayload::UserJoined { user_id: user },
        )
        .with_sequence(EventSequence::new(seq))
    }

    fn leave(cube: CubeId, user: UserId, seq: u64) -> Event {
        Event::new(
            cube,
            UserId::new(),
            EventPayload::UserLeft { user_id: user },
        )
        .with_sequence(EventSequence::new(seq))
    }

    fn create(cube: CubeId, path: &str, seq: u64) -> Event {
        Event::new(
            cube,
            UserId::new(),
            EventPayload::FileCreated { path: path.into() },
        )
        .with_sequence(EventSequence::new(seq))
    }

    #[test]
    fn presence_projection_tracks_members() {
        let mut p = PresenceProjection::new();
        let cube = CubeId::new();
        let alice = UserId::new();
        let bob = UserId::new();
        p.handle(&join(cube, alice, 1)).unwrap();
        p.handle(&join(cube, bob, 2)).unwrap();
        let members = p.members(cube);
        assert_eq!(members.len(), 2);
        assert!(members.contains(&alice) && members.contains(&bob));
        p.handle(&leave(cube, alice, 3)).unwrap();
        assert!(!p.is_present(cube, alice));
        assert!(p.is_present(cube, bob));
    }

    #[test]
    fn activity_feed_is_bounded_and_cube_filtered() {
        let mut feed = ActivityFeedProjection::new(3);
        let cube_a = CubeId::new();
        let cube_b = CubeId::new();
        feed.handle(&create(cube_a, "a1.rs", 1)).unwrap();
        feed.handle(&create(cube_a, "a2.rs", 2)).unwrap();
        feed.handle(&create(cube_b, "b1.rs", 3)).unwrap();
        feed.handle(&create(cube_a, "a3.rs", 4)).unwrap();
        // Capacity 3 -> the oldest, "a1.rs", is evicted.
        assert_eq!(feed.recent().len(), 3);
        let in_a = feed.recent_in(cube_a);
        assert_eq!(in_a.len(), 2);
    }
}
