use serde::{Deserialize, Serialize};

use crate::ids::{CubeId, FileId, UserId};
use crate::permission::Permission;
use crate::timestamp::Timestamp;

/// Stable user identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct User {
    pub id: UserId,
    pub display_name: String,
    pub permissions: Permission,
}

/// Presence status for a user in a Cube.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresenceStatus {
    Online,
    Away,
    Idle,
    Disconnected,
    Reconnecting,
}

/// A collaborator's presence state within a Cube.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Collaborator {
    pub user_id: UserId,
    pub display_name: String,
    pub status: PresenceStatus,
    pub active_file: Option<FileId>,
    pub cursor: Option<CursorPosition>,
    pub selection: Option<Selection>,
    pub color: u32, // Deterministic color from UserId
}

/// Cursor position in a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CursorPosition {
    pub file_id: FileId,
    pub position: u32, // Document position (byte offset or line/column)
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub user_id: UserId,
    pub timestamp: Timestamp,
}

/// Text selection range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    pub file_id: FileId,
    pub start: u32,
    pub end: u32,
    pub user_id: UserId,
    pub timestamp: Timestamp,
}

/// Activity event for the activity feed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityEvent {
    pub id: u64,
    pub cube_id: CubeId,
    pub user_id: UserId,
    pub user_name: String,
    pub event_type: ActivityEventType,
    pub timestamp: Timestamp,
    pub details: String,
}

/// Types of activity events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivityEventType {
    UserJoined,
    UserLeft,
    FileCreated,
    FileRenamed,
    FileDeleted,
    BuildStarted,
    BuildCompleted,
    TerminalShared,
    PermissionChanged,
}

/// Chat message in a Cube.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: u64,
    pub cube_id: CubeId,
    pub sender_id: UserId,
    pub sender_name: String,
    pub content: String,
    pub timestamp: Timestamp,
}

/// Pair programming mode state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PairProgrammingRole {
    Driver,
    Navigator,
}

/// Pair programming session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairSession {
    pub id: u64,
    pub cube_id: CubeId,
    pub driver: UserId,
    pub navigator: UserId,
    pub created_at: Timestamp,
}

/// Follow mode state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FollowState {
    Normal,
    Following(UserId),
    StoppedFollowing,
}

/// Build activity state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildActivity {
    pub cube_id: CubeId,
    pub user_id: UserId,
    pub command: String,
    pub status: BuildStatus,
    pub started_at: Timestamp,
}

/// Build status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuildStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// Terminal activity state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalActivity {
    pub cube_id: CubeId,
    pub user_id: UserId,
    pub terminal_id: u64,
    pub is_shared: bool,
    pub has_focus: bool,
    pub process_name: Option<String>,
}

impl User {
    pub fn new(id: UserId, display_name: String, permissions: Permission) -> Self {
        Self {
            id,
            display_name,
            permissions,
        }
    }

    /// Generates a deterministic color from the user ID.
    pub fn color(&self) -> u32 {
        let hash = self.id.as_uuid().as_u128();
        let r = ((hash >> 16) & 0xFF) as u32;
        let g = ((hash >> 8) & 0xFF) as u32;
        let b = (hash & 0xFF) as u32;
        // Ensure good contrast
        (r.saturating_add(128) % 256) << 16
            | (g.saturating_add(128) % 256) << 8
            | (b.saturating_add(128) % 256)
    }
}

impl Collaborator {
    pub fn from_user(user: User) -> Self {
        let color = user.color();
        Self {
            user_id: user.id,
            display_name: user.display_name,
            status: PresenceStatus::Online,
            active_file: None,
            cursor: None,
            selection: None,
            color,
        }
    }
}

impl CursorPosition {
    pub fn new(file_id: FileId, position: u32, user_id: UserId) -> Self {
        Self {
            file_id,
            position,
            line: None,
            column: None,
            user_id,
            timestamp: Timestamp::now(),
        }
    }
}

impl Selection {
    pub fn new(file_id: FileId, start: u32, end: u32, user_id: UserId) -> Self {
        Self {
            file_id,
            start,
            end,
            user_id,
            timestamp: Timestamp::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_color_is_deterministic() {
        let id = UserId::from_u64(42);
        let user1 = User::new(id, "Alice".into(), Permission::Editor);
        let user2 = User::new(id, "Alice".into(), Permission::Editor);
        assert_eq!(user1.color(), user2.color());
    }

    #[test]
    fn different_users_have_different_colors() {
        let user1 = User::new(UserId::from_u64(1), "Alice".into(), Permission::Editor);
        let user2 = User::new(UserId::from_u64(2), "Bob".into(), Permission::Editor);
        assert_ne!(user1.color(), user2.color());
    }
}
