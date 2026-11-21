use serde::{Deserialize, Serialize};

use crate::ids::{CubeId, SessionId, UserId};
use crate::timestamp::Timestamp;

/// State describing an authenticated connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionDescriptor {
    pub id: SessionId,
    pub user_id: UserId,
    pub username: String,
    pub cube_id: Option<CubeId>,
    pub connected_at: Timestamp,
    pub last_activity: Timestamp,
}

impl SessionDescriptor {
    #[must_use]
    pub fn new(user_id: UserId, username: impl Into<String>) -> Self {
        let now = Timestamp::now();
        Self {
            id: SessionId::new(),
            user_id,
            username: username.into(),
            cube_id: None,
            connected_at: now,
            last_activity: now,
        }
    }

    /// Records recent activity on this session.
    pub fn touch(&mut self) {
        self.last_activity = Timestamp::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_tracks_activity() {
        let mut session = SessionDescriptor::new(UserId::new(), "alice");
        let before = session.last_activity;
        std::thread::sleep(std::time::Duration::from_millis(2));
        session.touch();
        assert!(session.last_activity >= before);
        assert_eq!(session.cube_id, None);
    }

    #[test]
    fn session_serializes_through_json() {
        let session = SessionDescriptor::new(UserId::new(), "alice");
        let encoded = serde_json::to_string(&session).unwrap();
        let decoded: SessionDescriptor = serde_json::from_str(&encoded).unwrap();
        assert_eq!(session, decoded);
    }
}
