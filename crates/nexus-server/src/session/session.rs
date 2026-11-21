use std::time::Instant;

use nexus_core::ids::{ConnectionId, CubeId, SessionId, UserId};

use super::state::SessionState;

/// Server-side session record.
///
/// A *connection* is the network channel; a *session* is the logical user
/// interaction. They are kept separate so that a network disconnect can leave
/// a recoverable session behind (reconnection arrives in a later part).
#[derive(Debug, Clone)]
pub struct ServerSession {
    session_id: SessionId,
    user_id: UserId,
    username: String,
    connection_id: ConnectionId,
    cube_id: Option<CubeId>,
    state: SessionState,
    created_at: Instant,
    last_activity: Instant,
    last_received: Instant,
}

impl ServerSession {
    pub fn new(
        session_id: SessionId,
        user_id: UserId,
        username: impl Into<String>,
        connection_id: ConnectionId,
    ) -> Self {
        let now = Instant::now();
        Self {
            session_id,
            user_id,
            username: username.into(),
            connection_id,
            cube_id: None,
            state: SessionState::Created,
            created_at: now,
            last_activity: now,
            last_received: now,
        }
    }

    #[must_use]
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    #[must_use]
    pub const fn user_id(&self) -> UserId {
        self.user_id
    }

    #[must_use]
    pub fn username(&self) -> &str {
        &self.username
    }

    #[must_use]
    pub const fn connection_id(&self) -> ConnectionId {
        self.connection_id
    }

    #[must_use]
    pub const fn cube_id(&self) -> Option<CubeId> {
        self.cube_id
    }

    #[must_use]
    pub const fn state(&self) -> SessionState {
        self.state
    }

    #[must_use]
    pub const fn created_at(&self) -> Instant {
        self.created_at
    }

    #[must_use]
    pub const fn last_activity(&self) -> Instant {
        self.last_activity
    }

    #[must_use]
    pub const fn last_received(&self) -> Instant {
        self.last_received
    }

    pub fn set_cube(&mut self, cube_id: Option<CubeId>) {
        self.cube_id = cube_id;
    }

    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    pub fn note_received(&mut self) {
        self.last_received = Instant::now();
        self.touch();
    }

    pub fn transition_to(&mut self, state: SessionState) -> bool {
        if !self.state.can_transition_to(state) {
            return false;
        }
        self.state = state;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(connection_id: ConnectionId) -> ServerSession {
        ServerSession::new(SessionId::new(), UserId::new(), "alice", connection_id)
    }

    #[test]
    fn session_starts_created() {
        assert_eq!(sample(ConnectionId::new()).state(), SessionState::Created);
    }

    #[test]
    fn session_transitions_are_validated() {
        let connection = ConnectionId::new();
        let mut session = sample(connection);
        assert!(session.transition_to(SessionState::Authenticated));
        assert!(session.transition_to(SessionState::Active));
        assert!(
            !session.transition_to(SessionState::Created),
            "backwards move rejected"
        );
        assert!(session.transition_to(SessionState::Idle));
        assert!(session.transition_to(SessionState::Disconnecting));
        assert!(session.transition_to(SessionState::Disconnected));
        assert!(
            !session.transition_to(SessionState::Active),
            "terminal state"
        );
    }

    #[test]
    fn tracks_cube_membership() {
        let mut session = sample(ConnectionId::new());
        let cube = CubeId::new();
        session.set_cube(Some(cube));
        assert_eq!(session.cube_id(), Some(cube));
        session.set_cube(None);
        assert_eq!(session.cube_id(), None);
    }

    #[test]
    fn note_received_touches_activity() {
        let mut session = sample(ConnectionId::new());
        let before = session.last_activity();
        session.note_received();
        assert!(session.last_activity() >= before);
    }
}
