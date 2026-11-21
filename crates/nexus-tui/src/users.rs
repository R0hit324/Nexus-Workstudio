//! Users panel: presence of everyone in the current cube.

use nexus_core::ids::UserId;

/// Presence status of a user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserStatus {
    Online,
    Busy,
    Away,
    Offline,
}

/// A member of the current cube.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserPresence {
    pub user_id: UserId,
    pub name: String,
    pub status: UserStatus,
    /// Idle seconds for away-detection.
    pub idle_seconds: u64,
}

/// Bounded list of cube members, sorted by name for a stable UI.
#[derive(Debug, Clone, Default)]
pub struct UsersPanelState {
    members: Vec<UserPresence>,
}

impl UsersPanelState {
    /// Inserts/replaces a member; keeps the list sorted by name.
    pub fn upsert(&mut self, user: UserPresence) {
        if let Some(existing) = self.members.iter_mut().find(|m| m.user_id == user.user_id) {
            *existing = user;
        } else {
            self.members.push(user);
        }
        self.members.sort_by(|a, b| a.name.cmp(&b.name));
    }

    /// Removes a user (on disconnect).
    pub fn remove(&mut self, user_id: UserId) {
        self.members.retain(|m| m.user_id != user_id);
    }

    #[must_use]
    pub fn members(&self) -> &[UserPresence] {
        &self.members
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// A member's name, if known.
    #[must_use]
    pub fn name_of(&self, user_id: UserId) -> Option<&str> {
        self.members
            .iter()
            .find(|m| m.user_id == user_id)
            .map(|m| m.name.as_str())
    }

    /// Total members, including bots/placeholders.
    #[must_use]
    pub fn len(&self) -> usize {
        self.members.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(id: u64, name: &str) -> UserPresence {
        UserPresence {
            user_id: UserId::from_u64(id),
            name: name.to_owned(),
            status: UserStatus::Online,
            idle_seconds: 0,
        }
    }

    #[test]
    fn members_are_sorted_by_name() {
        let mut s = UsersPanelState::default();
        s.upsert(user(2, "beta"));
        s.upsert(user(1, "alpha"));
        s.upsert(user(3, "charlie"));
        let names: Vec<&str> = s.members().iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta", "charlie"]);
    }

    #[test]
    fn upsert_replaces_in_place() {
        let mut s = UsersPanelState::default();
        s.upsert(user(1, "alpha"));
        s.upsert(user(1, "Alpha"));
        assert_eq!(s.len(), 1);
        assert_eq!(s.name_of(UserId::from_u64(1)), Some("Alpha"));
    }

    #[test]
    fn remove_drops_member() {
        let mut s = UsersPanelState::default();
        s.upsert(user(1, "alpha"));
        s.remove(UserId::from_u64(1));
        assert!(s.is_empty());
        assert_eq!(s.name_of(UserId::from_u64(1)), None);
    }

    #[test]
    fn name_lookup_only_known() {
        let mut s = UsersPanelState::default();
        assert_eq!(s.name_of(UserId::from_u64(9)), None);
        s.upsert(user(9, "zed"));
        assert_eq!(s.name_of(UserId::from_u64(9)), Some("zed"));
    }
}
