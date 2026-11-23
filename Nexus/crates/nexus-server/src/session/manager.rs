use std::collections::HashMap;
use std::sync::Mutex;

use nexus_core::ids::{ConnectionId, CubeId, SessionId, UserId};

use super::session::ServerSession;
use super::state::SessionState;

/// Owns the lifecycle of all server-side sessions.
///
/// The manager is the *only* module allowed to mutate
/// [`SessionState`] or create/destroy sessions. Other modules build on top of
/// its API instead of touching a shared `Session` directly.
pub struct SessionManager {
    by_id: Mutex<HashMap<SessionId, ServerSession>>,
    by_connection: Mutex<HashMap<ConnectionId, SessionId>>,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            by_id: Mutex::new(HashMap::new()),
            by_connection: Mutex::new(HashMap::new()),
        }
    }

    /// Prepares a session for a fresh connection. The session starts in
    /// [`SessionState::Created`] and advances to `Authenticated` after the
    /// authenticator succeeds.
    #[must_use]
    pub fn create(
        &self,
        connection_id: ConnectionId,
        user_id: UserId,
        username: impl Into<String>,
    ) -> ServerSession {
        let session = ServerSession::new(SessionId::new(), user_id, username, connection_id);
        let session_id = session.session_id();
        self.by_connection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(connection_id, session_id);
        self.by_id
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(session_id, session.clone());
        session
    }

    pub fn transition_to(&self, session_id: SessionId, state: SessionState) -> bool {
        self.with_mut(session_id, |session| session.transition_to(state))
    }

    /// Commits the session state after authentication succeeded.
    pub fn mark_authenticated(&self, session_id: SessionId) -> bool {
        self.transition_to(session_id, SessionState::Authenticated)
    }

    /// Attaches the user to a Cube. The session must be Authenticated or Idle.
    pub fn join_cube(&self, session_id: SessionId, cube_id: CubeId) -> bool {
        self.with_mut(session_id, |session| {
            session.set_cube(Some(cube_id));
            match session.state() {
                SessionState::Authenticated | SessionState::Active | SessionState::Idle => {
                    session.transition_to(SessionState::Active)
                }
                _ => false,
            }
        })
    }

    /// Detaches the session from its Cube, returning to Authenticated.
    pub fn leave_cube(&self, session_id: SessionId) -> bool {
        self.with_mut(session_id, |session| {
            session.set_cube(None);
            session.transition_to(SessionState::Authenticated)
        })
    }

    pub fn touch(&self, session_id: SessionId) {
        self.with_mut(session_id, |session| {
            session.touch();
            true
        });
    }

    pub fn note_received(&self, session_id: SessionId) {
        self.with_mut(session_id, |session| {
            session.note_received();
            true
        });
    }

    /// Marks the session as disconnecting. On the next cleanup pass the
    /// session is destroyed.
    pub fn begin_disconnect(&self, session_id: SessionId) -> bool {
        self.transition_to(session_id, SessionState::Disconnecting)
    }

    /// Destroys the session and all maps pointing at it. Returns the session.
    pub fn destroy(&self, session_id: SessionId) -> Option<ServerSession> {
        let mut by_id = self.by_id.lock().unwrap_or_else(|p| p.into_inner());
        let session = by_id.remove(&session_id)?;
        self.by_connection
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|_, id| *id != session_id);
        Some(session)
    }

    /// Returns a clone of the session, or `None` when unknown.
    pub fn get(&self, session_id: SessionId) -> Option<ServerSession> {
        self.with(session_id, Clone::clone)
    }

    pub fn session_for_connection(&self, connection_id: ConnectionId) -> Option<ServerSession> {
        let id = self
            .by_connection
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&connection_id)
            .copied()?;
        self.get(id)
    }

    /// Whether any live session belongs to the given user.
    pub fn user_has_session(&self, user_id: UserId) -> bool {
        self.by_id
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .any(|session| session.user_id() == user_id)
    }

    pub fn len(&self) -> usize {
        self.by_id.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Total sessions that are currently in a non-terminal state.
    pub fn active_count(&self) -> usize {
        self.by_id
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .filter(|s| s.state() != SessionState::Disconnected)
            .count()
    }

    fn with<T>(&self, session_id: SessionId, f: impl FnOnce(&ServerSession) -> T) -> Option<T> {
        self.by_id
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&session_id)
            .map(f)
    }

    fn with_mut(&self, session_id: SessionId, f: impl FnOnce(&mut ServerSession) -> bool) -> bool {
        self.by_id
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_mut(&session_id)
            .is_some_and(f)
    }
}

// `ServerSession` derives `Clone` (cheap: ids are `Copy`, only the username
// is a `String`); `get` returns a clone so callers never hold a lock.

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SessionManager {
        SessionManager::new()
    }

    #[test]
    fn create_get_destroy_roundtrip() {
        let manager = sample();
        let connection = ConnectionId::new();
        let created = manager.create(connection, UserId::new(), "alice");
        let id = created.session_id();
        assert_eq!(manager.get(id).map(|s| s.session_id()), Some(id));
        assert_eq!(
            manager
                .session_for_connection(connection)
                .map(|s| s.session_id()),
            Some(id)
        );

        manager.destroy(id);
        assert!(manager.get(id).is_none());
        assert!(manager.session_for_connection(connection).is_none());
        assert_eq!(manager.len(), 0);
    }

    #[test]
    fn authenticates_and_joins_cube() {
        let manager = sample();
        let connection = ConnectionId::new();
        let created = manager.create(connection, UserId::new(), "alice");
        let id = created.session_id();
        let cube = CubeId::new();

        assert!(manager.mark_authenticated(id));
        assert!(manager.join_cube(id, cube));
        assert_eq!(manager.get(id).unwrap().cube_id(), Some(cube));
        assert_eq!(manager.get(id).unwrap().state(), SessionState::Active);

        assert!(manager.leave_cube(id));
        assert_eq!(manager.get(id).unwrap().cube_id(), None);
        assert_eq!(
            manager.get(id).unwrap().state(),
            SessionState::Authenticated
        );
    }

    #[test]
    fn destroy_cleans_connection_mapping() {
        let manager = sample();
        let connection = ConnectionId::new();
        let created = manager.create(connection, UserId::new(), "bob");
        let id = created.session_id();
        manager.begin_disconnect(id);
        manager.destroy(id);
        assert!(manager.session_for_connection(connection).is_none());
        assert_eq!(manager.active_count(), 0);
    }

    #[test]
    fn state_transitions_are_enforced() {
        let manager = sample();
        let created = manager.create(ConnectionId::new(), UserId::new(), "carol");
        let id = created.session_id();
        // Can't jump straight to Active without authenticating.
        assert!(!manager.join_cube(id, CubeId::new()));
        assert!(manager.begin_disconnect(id));
        // After disconnect started we cannot resurrect via join.
        assert!(!manager.join_cube(id, CubeId::new()));
    }
}
