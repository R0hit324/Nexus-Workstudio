use nexus_core::event::{Event, EventCategory, EventSequence, EventType};
use nexus_core::ids::{CubeId, SessionId, UserId};

/// A composable predicate over events (spec section 26 and 27).
///
/// Filters are created through narrow builders and combine with `and`/`or`/
/// `not`, so coarse collections ("all workspace events") and precise
/// subscriptions ("cursor moves by user X in cube C") share one type.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum EventFilter {
    /// Accept everything.
    #[default]
    All,
    /// Accept nothing.
    None,
    /// Accept only events of the given broad category.
    Category(EventCategory),
    /// Accept only events of one of the listed concrete types.
    Type(Vec<EventType>),
    /// Accept only events for this Cube.
    Cube(CubeId),
    /// Accept only events authored by this user.
    Actor(UserId),
    /// Accept only events originating from this session.
    Session(SessionId),
    /// Accept events at or above this sequence (used for replay catch-up).
    FromSequence(EventSequence),
    /// Logical connectors.
    And(Box<EventFilter>, Box<EventFilter>),
    Or(Box<EventFilter>, Box<EventFilter>),
    Not(Box<EventFilter>),
}

impl EventFilter {
    /// A filter accepting no events.
    #[must_use]
    pub fn never() -> Self {
        Self::None
    }

    /// A filter accepting every event.
    #[must_use]
    pub fn all() -> Self {
        Self::All
    }

    /// Only events of `category`.
    #[must_use]
    pub fn category(category: EventCategory) -> Self {
        Self::Category(category)
    }

    /// Only events whose type is one of `types`.
    #[must_use]
    pub fn types(types: impl IntoIterator<Item = EventType>) -> Self {
        Self::Type(types.into_iter().collect())
    }

    /// Only events for `cube`.
    #[must_use]
    pub fn cube(cube: CubeId) -> Self {
        Self::Cube(cube)
    }

    /// Only events authored by `actor`.
    #[must_use]
    pub fn actor(actor: UserId) -> Self {
        Self::Actor(actor)
    }

    /// Only events originating from `session`.
    #[must_use]
    pub fn session(session: SessionId) -> Self {
        Self::Session(session)
    }

    /// Matches `event`.
    #[must_use]
    pub fn matches(&self, event: &Event) -> bool {
        match self {
            Self::All => true,
            Self::None => false,
            Self::Category(c) => event.category() == *c,
            Self::Type(types) => types.contains(&event.event_type()),
            Self::Cube(cube) => &event.cube_id() == cube,
            Self::Actor(actor) => &event.actor() == actor,
            Self::Session(session) => event.session() == Some(*session),
            Self::FromSequence(from) => event.sequence() >= *from,
            Self::And(l, r) => l.matches(event) && r.matches(event),
            Self::Or(l, r) => l.matches(event) || r.matches(event),
            Self::Not(inner) => !inner.matches(event),
        }
    }

    /// Logical AND.
    #[must_use]
    pub fn and(self, other: Self) -> Self {
        Self::And(Box::new(self), Box::new(other))
    }

    /// Logical OR.
    #[must_use]
    pub fn or(self, other: Self) -> Self {
        Self::Or(Box::new(self), Box::new(other))
    }

    /// Logical negation.
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn not(self) -> Self {
        Self::Not(Box::new(self))
    }

    /// A coarse "is this filter likely to match at all" hint used to skip
    /// whole cubes when the filter constrains the cube.
    #[must_use]
    pub fn cubehint(&self) -> Option<CubeId> {
        match self {
            Self::Cube(c) => Some(*c),
            Self::And(l, r) => l.cubehint().or_else(|| r.cubehint()),
            Self::Or(l, r) => l.cubehint().or_else(|| r.cubehint()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::event::{Event, EventPayload};
    use nexus_core::ids::{CubeId, UserId};

    fn cursor_event(cube: CubeId, actor: UserId) -> Event {
        Event::new(
            cube,
            actor,
            EventPayload::CursorMoved {
                path: "a.rs".into(),
                line: 1,
                column: 2,
            },
        )
        .with_sequence(EventSequence::new(1))
    }

    fn join_event(cube: CubeId, actor: UserId) -> Event {
        Event::new(cube, actor, EventPayload::UserJoined { user_id: actor })
    }

    #[test]
    fn filters_by_category_and_type() {
        let cube = CubeId::new();
        let actor = UserId::new();
        let cursor = cursor_event(cube, actor);
        let joined = join_event(cube, actor);

        assert!(EventFilter::category(EventCategory::Workspace).matches(&cursor));
        assert!(!EventFilter::category(EventCategory::Collaboration).matches(&cursor));
        assert!(EventFilter::category(EventCategory::Collaboration).matches(&joined));

        assert!(EventFilter::types([EventType::CursorMoved]).matches(&cursor));
        assert!(!EventFilter::types([EventType::CursorMoved]).matches(&joined));
    }

    #[test]
    fn filters_combine() {
        let cube = CubeId::new();
        let actor = UserId::new();
        let other = UserId::new();
        let cursor = cursor_event(cube, actor);

        let mine = EventFilter::actor(actor);
        let theirs = EventFilter::actor(other);
        assert!(mine.matches(&cursor));
        assert!(!theirs.matches(&cursor));

        let combined = EventFilter::category(EventCategory::Workspace).and(mine.clone());
        assert!(combined.matches(&cursor));

        let or = mine.clone().or(theirs);
        assert!(or.matches(&cursor));

        assert!(!mine.not().matches(&cursor));
        assert!(EventFilter::cube(cube).matches(&cursor));
        assert_eq!(EventFilter::cube(cube).cubehint(), Some(cube));
    }

    #[test]
    fn filter_never_and_all() {
        let event = cursor_event(CubeId::new(), UserId::new());
        assert!(EventFilter::all().matches(&event));
        assert!(!EventFilter::never().matches(&event));
    }
}
