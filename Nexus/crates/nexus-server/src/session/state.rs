use std::fmt;

/// Server-side lifecycle state for a session. Every transition is validated
/// via [`SessionState::can_transition_to`] so that managers and connection
/// tasks never observe an inconsistent state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionState {
    /// A connection has been accepted but authentication has not completed.
    Created,
    /// The user has been authenticated. The session is visible but the user
    /// has not joined a Cube.
    Authenticated,
    /// Connected and joined a Cube, actively exchanging messages.
    Active,
    /// Connected but idle (not currently inside any Cube or silence between
    /// heartbeats). A temporary holding state so that rejoining a Cube
    /// after leaving one does not bounce through Authenticated.
    Idle,
    /// Graceful disconnect in progress. The server is flushing any remaining
    /// writes and tearing down the connection.
    Disconnecting,
    /// Terminal state. The session is logically gone.
    Disconnected,
}

impl SessionState {
    /// Returns `true` when a transition from `self` to `to` is permitted.
    #[must_use]
    pub fn can_transition_to(self, to: SessionState) -> bool {
        matches!(
            (self, to),
            (Self::Created, Self::Authenticated)
                | (Self::Created, Self::Disconnecting)
                | (Self::Authenticated, Self::Active)
                | (Self::Authenticated, Self::Idle)
                | (Self::Authenticated, Self::Disconnecting)
                | (Self::Active, Self::Idle)
                | (Self::Active, Self::Authenticated)
                | (Self::Active, Self::Disconnecting)
                | (Self::Idle, Self::Active)
                | (Self::Idle, Self::Authenticated)
                | (Self::Idle, Self::Disconnecting)
                | (Self::Disconnecting, Self::Disconnected)
        )
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Authenticated => "authenticated",
            Self::Active => "active",
            Self::Idle => "idle",
            Self::Disconnecting => "disconnecting",
            Self::Disconnected => "disconnected",
        }
    }
}

impl fmt::Display for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_transitions() {
        assert!(SessionState::Created.can_transition_to(SessionState::Authenticated));
        assert!(SessionState::Created.can_transition_to(SessionState::Disconnecting));
        assert!(SessionState::Authenticated.can_transition_to(SessionState::Active));
        assert!(SessionState::Authenticated.can_transition_to(SessionState::Idle));
        assert!(SessionState::Active.can_transition_to(SessionState::Idle));
        assert!(SessionState::Idle.can_transition_to(SessionState::Active));
        assert!(SessionState::Disconnecting.can_transition_to(SessionState::Disconnected));
    }

    #[test]
    fn invalid_transitions_rejected() {
        assert!(!SessionState::Created.can_transition_to(SessionState::Active));
        assert!(!SessionState::Authenticated.can_transition_to(SessionState::Created));
        assert!(!SessionState::Authenticated.can_transition_to(SessionState::Disconnected));
        assert!(!SessionState::Active.can_transition_to(SessionState::Created));
        assert!(!SessionState::Disconnected.can_transition_to(SessionState::Active));
        assert!(!SessionState::Idle.can_transition_to(SessionState::Created));
    }
}
