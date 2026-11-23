use std::sync::Arc;

use nexus_core::event::Event;
use nexus_core::ids::{CubeId, SessionId};
use nexus_protocol::message::Message;

use super::bus::EventBus;
use crate::connection::ConnectionManager;
use crate::session::SessionManager;

/// Scales events from the domain layer out to the wire.
///
/// Two responsibilities:
/// 1. publish to the [`EventBus`] so internal subscribers (audit, Part 4
///    collaboration engines) receive every event;
/// 2. broadcast the corresponding `Message::Event` to every live connection
///    of a Cube, using the connection manager's drop-when-full policy.
pub struct EventRouter {
    event_bus: Arc<EventBus>,
    engine: Option<Arc<nexus_events::bus::EventBus>>,
    connection_manager: Arc<ConnectionManager>,
    session_manager: Arc<SessionManager>,
}

impl EventRouter {
    pub fn new(
        event_bus: Arc<EventBus>,
        connection_manager: Arc<ConnectionManager>,
        session_manager: Arc<SessionManager>,
    ) -> Self {
        Self {
            event_bus,
            engine: None,
            connection_manager,
            session_manager,
        }
    }

    /// Constructs a router that runs the full Part 4 creation pipeline
    /// (sequence assignment, persistence, metrics, fan-out) before
    /// broadcasting to connected clients.
    pub fn with_engine(
        event_bus: Arc<EventBus>,
        engine: Arc<nexus_events::bus::EventBus>,
        connection_manager: Arc<ConnectionManager>,
        session_manager: Arc<SessionManager>,
    ) -> Self {
        Self {
            event_bus,
            engine: Some(engine),
            connection_manager,
            session_manager,
        }
    }

    /// Publishes `event` to every subscriber and wires it to Cube members.
    ///
    /// When a Part 4 [`nexus_events::bus::EventBus`] engine is wired in,
    /// the event is sequenced, persisted (if durable), and metriced first;
    /// the broadcast to connected clients uses the authoritative,
    /// fully-sequenced event record.
    ///
    /// `exclude_session` (when provided) suppresses delivery to that session —
    /// used to avoid echoing an action back to its originator.
    pub fn dispatch(
        &self,
        cube_id: CubeId,
        event: Event,
        exclude_session: Option<SessionId>,
    ) -> Result<(), nexus_events::Error> {
        let event = match &self.engine {
            Some(engine) => engine.publish(event)?.event,
            None => event,
        };
        self.event_bus.publish(cube_id, event.clone());
        let message = Message::Event { event };
        let excluded = exclude_session.and_then(|sid| {
            self.session_manager
                .get(sid)
                .map(|session| session.connection_id())
        });
        let _ = self
            .connection_manager
            .broadcast_to_cube_except(cube_id, message, excluded);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::ConnectionManager;
    use nexus_core::event::EventPayload;
    use nexus_core::ids::ConnectionId;

    fn event(cube_id: CubeId) -> Event {
        Event::new(
            cube_id,
            nexus_core::ids::UserId::new(),
            EventPayload::UserJoined {
                user_id: nexus_core::ids::UserId::new(),
            },
        )
    }

    /// Creates a session inside `cube` with a registered connection.
    fn member(
        sessions: &SessionManager,
        connections: &ConnectionManager,
        cube: CubeId,
    ) -> (SessionId, tokio::sync::mpsc::Receiver<Message>) {
        let session = sessions.create(ConnectionId::new(), nexus_core::ids::UserId::new(), "alice");
        let session_id = session.session_id();
        assert!(sessions.mark_authenticated(session_id));
        assert!(sessions.join_cube(session_id, cube));
        let rx = connections.register(
            session.connection_id(),
            "127.0.0.1:9000".parse().unwrap(),
            8,
        );
        connections.bind_session(session.connection_id(), session_id);
        connections.bind_cube(session.connection_id(), Some(cube));
        (session_id, rx)
    }

    #[tokio::test]
    async fn dispatch_broadcasts_to_members() {
        let event_bus = Arc::new(EventBus::new());
        let connections = Arc::new(ConnectionManager::new());
        let sessions = Arc::new(SessionManager::new());
        let router = EventRouter::new(event_bus, connections.clone(), sessions.clone());

        let cube = CubeId::new();
        let (_, mut rx) = member(&sessions, &connections, cube);

        router.dispatch(cube, event(cube), None).unwrap();
        assert!(matches!(rx.recv().await, Some(Message::Event { .. })));
    }

    #[tokio::test]
    async fn exclude_session_skips_originator() {
        let event_bus = Arc::new(EventBus::new());
        let connections = Arc::new(ConnectionManager::new());
        let sessions = Arc::new(SessionManager::new());
        let router = EventRouter::new(event_bus, connections.clone(), sessions.clone());

        let cube = CubeId::new();
        let (session_id, mut rx) = member(&sessions, &connections, cube);

        router
            .dispatch(cube, event(cube), Some(session_id))
            .unwrap();
        assert!(
            rx.try_recv().is_err(),
            "originator must not receive its own event"
        );
    }

    #[tokio::test]
    async fn other_member_still_receives() {
        let event_bus = Arc::new(EventBus::new());
        let connections = Arc::new(ConnectionManager::new());
        let sessions = Arc::new(SessionManager::new());
        let router = EventRouter::new(event_bus, connections.clone(), sessions.clone());

        let cube = CubeId::new();
        let bob_session =
            sessions.create(ConnectionId::new(), nexus_core::ids::UserId::new(), "bob");
        let bob_id = bob_session.session_id();
        assert!(sessions.mark_authenticated(bob_id));
        assert!(sessions.join_cube(bob_id, cube));
        let mut bob_rx = connections.register(
            bob_session.connection_id(),
            "127.0.0.1:9001".parse().unwrap(),
            8,
        );
        connections.bind_session(bob_session.connection_id(), bob_id);
        connections.bind_cube(bob_session.connection_id(), Some(cube));

        let (alice_id, _alice_rx) = member(&sessions, &connections, cube);
        assert_ne!(alice_id, bob_id);

        router.dispatch(cube, event(cube), Some(alice_id)).unwrap();
        assert!(matches!(bob_rx.recv().await, Some(Message::Event { .. })));
    }

    #[tokio::test]
    async fn engine_sequences_and_persists_event() {
        let event_bus = Arc::new(EventBus::new());
        let engine = Arc::new(nexus_events::bus::EventBus::default());
        let connections = Arc::new(ConnectionManager::new());
        let sessions = Arc::new(SessionManager::new());
        let router = EventRouter::with_engine(
            event_bus,
            engine.clone(),
            connections.clone(),
            sessions.clone(),
        );

        let cube = CubeId::new();
        let (_, mut rx) = member(&sessions, &connections, cube);

        let e = Event::new(
            cube,
            nexus_core::ids::UserId::new(),
            EventPayload::PermissionChanged {
                user_id: nexus_core::ids::UserId::new(),
            },
        );
        router.dispatch(cube, e, None).unwrap();
        let msg = rx.recv().await.unwrap();
        if let Message::Event { event } = msg {
            assert!(event.is_sequenced(), "event should be stamped by engine");
            assert_eq!(event.sequence().value(), 1);
            assert_eq!(
                engine
                    .replay(cube, nexus_core::event::EventSequence::UNASSIGNED)
                    .unwrap()
                    .len(),
                1
            );
        } else {
            panic!("expected Message::Event");
        }
    }
}
