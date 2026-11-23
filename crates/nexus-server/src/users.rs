use std::collections::HashMap;
use std::sync::Mutex;

use nexus_core::error::Error;
use nexus_core::ids::UserId;
use nexus_core::timestamp::Timestamp;

/// How the user authenticated. Part 3 ships trust-based authentication only;
/// the variant is stored so that later parts (tokens, keys, MFA) can extend
/// without changing the registry shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticationMethod {
    Trust,
}

/// A registered user account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    id: UserId,
    username: String,
    display_name: String,
    auth_method: AuthenticationMethod,
    created_at: Timestamp,
}

impl User {
    #[must_use]
    pub fn new(id: UserId, username: impl Into<String>, display_name: impl Into<String>) -> Self {
        Self {
            id,
            username: username.into(),
            display_name: display_name.into(),
            auth_method: AuthenticationMethod::Trust,
            created_at: Timestamp::now(),
        }
    }

    #[must_use]
    pub const fn id(&self) -> UserId {
        self.id
    }

    #[must_use]
    pub fn username(&self) -> &str {
        &self.username
    }

    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    #[must_use]
    pub const fn auth_method(&self) -> AuthenticationMethod {
        self.auth_method
    }

    #[must_use]
    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }
}

/// Username normalization rules shared by the authenticator and tests.
///
/// Accepts 1-64 characters of `[A-Za-z0-9._-]` after trimming, lower-cased
/// for canonical keying so `Alice` and `alice` cannot collide.
pub fn normalize_username(raw: &str) -> Option<String> {
    let trimmed = raw.trim().to_ascii_lowercase();
    if trimmed.is_empty() || trimmed.len() > 64 {
        return None;
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return None;
    }
    Some(trimmed)
}

/// The authoritative set of users known to the server.
///
/// Users are created idempotently on first authentication: the same username
/// always maps to the same [`UserId`].
pub struct UserRegistry {
    by_username: Mutex<HashMap<String, User>>,
}

impl Default for UserRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl UserRegistry {
    pub fn new() -> Self {
        Self {
            by_username: Mutex::new(HashMap::new()),
        }
    }

    /// Resolves a username to a user, creating it on first sight.
    ///
    /// Returns an [`ErrorCode::InvalidRequest`] error when the username does
    /// not meet the profile rules.
    pub fn get_or_register(
        &self,
        raw_username: &str,
        display_name: &str,
    ) -> nexus_core::Result<User> {
        let normalized = normalize_username(raw_username)
            .ok_or_else(|| Error::invalid_request("username contains unsupported characters"))?;
        let display = if display_name.trim().is_empty() {
            normalized.clone()
        } else {
            display_name.trim().to_owned()
        };

        let mut users = self.by_username.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(existing) = users.get(&normalized) {
            return Ok(existing.clone());
        }
        let user = User::new(UserId::new(), normalized.clone(), display);
        users.insert(normalized, user.clone());
        Ok(user)
    }

    pub fn get(&self, user_id: UserId) -> Option<User> {
        self.by_username
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .find(|user| user.id() == user_id)
            .cloned()
    }

    pub fn by_username(&self, username: &str) -> Option<User> {
        let normalized = normalize_username(username)?;
        self.by_username
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&normalized)
            .cloned()
    }

    pub fn len(&self) -> usize {
        self.by_username
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_rules() {
        assert_eq!(normalize_username("  Alice  ").as_deref(), Some("alice"));
        assert_eq!(
            normalize_username("Bobby.Tables").as_deref(),
            Some("bobby.tables")
        );
        assert_eq!(normalize_username("").as_deref(), None);
        assert_eq!(
            normalize_username("a b").as_deref(),
            None,
            "space is invalid"
        );
        assert_eq!(
            normalize_username(&"x".repeat(65)).as_deref(),
            None,
            "too long"
        );
    }

    #[test]
    fn registers_idempotently() {
        let registry = UserRegistry::new();
        let first = registry.get_or_register("alice", "Alice").unwrap();
        let second = registry.get_or_register("ALICE", "Different").unwrap();
        assert_eq!(first.id(), second.id());
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn lookup_by_id_and_username() {
        let registry = UserRegistry::new();
        let user = registry.get_or_register("bob", "Bob Builder").unwrap();
        assert_eq!(registry.by_username("BOB").unwrap().id(), user.id());
        assert_eq!(
            registry.get(user.id()).unwrap().display_name(),
            "Bob Builder"
        );
        assert_eq!(registry.by_username("carol"), None);
    }

    #[test]
    fn rejects_bad_username() {
        let registry = UserRegistry::new();
        let err = registry.get_or_register("bad/name", "x").unwrap_err();
        assert_eq!(err.code(), nexus_core::error::ErrorCode::InvalidRequest);
    }
}
