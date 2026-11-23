//! AI session layer (`§11`).
//!
//! Sessions own conversation identity and boundaries. A *private* session is
//! readable only by its owner — the manager enforces this so context that the
//! user gathered privately never leaks into other users' requests (`§49`).

use crate::error::{AiError, AiErrorKind, AiResult};
use crate::message::AiMessage;
use crate::model::{ModelId, ProviderId};
use nexus_core::ids::{CubeId, UserId};
use nexus_core::version::Version as CoreVersion;
use nexus_core::version::PROTOCOL_VERSION;
use std::collections::BTreeMap;
use std::time::Instant;

/// Identifier of one AI conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AiSessionId(u64);

impl AiSessionId {
    #[must_use]
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    #[must_use]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }
}

/// Visibility of a session (`§11`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiSessionScope {
    /// Only the owner sees/uses it.
    Private,
    /// Bound to a Cube; still owner-controlled.
    Cube(CubeId),
    /// Future: explicit sharing between named collaborators.
    Shared,
}

/// One conversation attached to owner/scope/config.
#[derive(Debug, Clone)]
pub struct AiSession {
    pub id: AiSessionId,
    pub owner: UserId,
    pub scope: AiSessionScope,
    pub model: ModelId,
    pub provider: ProviderId,
    pub history: Vec<AiMessage>,
    pub created: Instant,
    pub updated: Instant,
    /// Protocol version pinned when the session opened (snapshot/recovery
    /// integration pins the conversation format).
    pub protocol_version: CoreVersion,
}

impl AiSession {
    /// Appends a message, bumping `updated` and bounding history length.
    /// Returns `false` when `GET` — i.e. when the caller is not the owner and
    /// the session is private.
    pub fn push_message(&mut self, message: AiMessage, max_history: usize) {
        self.history.push(message);
        if max_history > 0 {
            let overflow = self.history.len().saturating_sub(max_history);
            let _ = self.history.drain(..overflow);
        }
        self.updated = Instant::now();
    }

    /// Whether `user` may view/continue this conversation (`§49`).
    #[must_use]
    pub fn accessible_by(&self, user: UserId) -> bool {
        user == self.owner
    }
}

/// Manages conversations for all users.
#[derive(Debug, Default)]
pub struct AiSessionManager {
    sessions: BTreeMap<AiSessionId, AiSession>,
    next_id: u64,
    default_max_history: usize,
}

impl AiSessionManager {
    /// Bounded history keeps memory predictable (`§72`).
    pub const DEFAULT_MAX_HISTORY: usize = 64;

    #[must_use]
    pub fn new() -> Self {
        Self {
            sessions: BTreeMap::new(),
            next_id: 1,
            default_max_history: Self::DEFAULT_MAX_HISTORY,
        }
    }

    /// Opens a session; the id allocator starts at 1 and never reuses ids.
    pub fn open(
        &mut self,
        owner: UserId,
        scope: AiSessionScope,
        provider: ProviderId,
        model: ModelId,
    ) -> AiSessionId {
        let id = AiSessionId::new(self.next_id);
        self.next_id += 1;
        let now = Instant::now();
        self.sessions.insert(
            id,
            AiSession {
                id,
                owner,
                scope,
                model,
                provider,
                history: Vec::new(),
                created: now,
                updated: now,
                protocol_version: PROTOCOL_VERSION,
            },
        );
        id
    }
    /// The session, only when `user` may access it (`§49`).
    #[must_use]
    pub fn get(&self, id: AiSessionId, user: UserId) -> Option<&AiSession> {
        self.sessions
            .get(&id)
            .filter(|session| session.accessible_by(user))
    }

    /// Mutable variant with the same isolation rule.
    #[must_use]
    pub fn get_mut(&mut self, id: AiSessionId, user: UserId) -> Option<&mut AiSession> {
        self.sessions
            .get_mut(&id)
            .filter(|session| session.accessible_by(user))
    }

    /// Appends a message on behalf of `user`; errors on foreign/idle sessions.
    ///
    /// # Errors
    /// `PermissionDenied` for unknown or foreign sessions.
    pub fn record(&mut self, id: AiSessionId, user: UserId, message: AiMessage) -> AiResult<()> {
        let session = self
            .sessions
            .get_mut(&id)
            .filter(|session| session.accessible_by(user))
            .ok_or_else(|| {
                AiError::new(
                    AiErrorKind::PermissionDenied,
                    "session.record",
                    format!("session {id:?} not accessible by caller"),
                )
            })?;
        session.push_message(message, self.default_max_history);
        Ok(())
    }

    /// Sessions visible to `user`.
    #[must_use]
    pub fn sessions_of(&self, user: UserId) -> Vec<&AiSession> {
        self.sessions
            .values()
            .filter(|session| session.accessible_by(user))
            .collect()
    }

    /// Closes a session; the conversation is dropped, not archived (`§51`
    /// persistence policy is opt-in per client, handled in that slice).
    pub fn close(&mut self, id: AiSessionId, user: UserId) -> AiResult<()> {
        let session = self
            .sessions
            .get(&id)
            .filter(|session| session.accessible_by(user))
            .ok_or_else(|| {
                AiError::new(
                    AiErrorKind::PermissionDenied,
                    "session.close",
                    format!("session {id:?} not accessible by caller"),
                )
            })?
            .clone();
        self.sessions.remove(&session.id);
        Ok(())
    }

    /// Number of live sessions (bounded by close discipline).
    #[must_use]
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Whether no sessions are open.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uid(n: u64) -> UserId {
        UserId::from_u64(n)
    }

    #[test]
    fn open_allocates_monotonic_ids_and_bounds_history() {
        let mut manager = AiSessionManager::new();
        let alice = uid(1);
        let id = manager.open(
            alice,
            AiSessionScope::Private,
            ProviderId::new("local"),
            ModelId::new("tiny"),
        );
        assert_eq!(id.as_u64(), 1);
        let second = manager.open(
            alice,
            AiSessionScope::Private,
            ProviderId::new("local"),
            ModelId::new("tiny"),
        );
        assert_eq!(second.as_u64(), 2);

        for i in 0..80 {
            manager
                .record(id, alice, AiMessage::user(format!("msg {i}")))
                .unwrap();
        }
        let session = manager.get(id, alice).unwrap();
        assert_eq!(session.history.len(), AiSessionManager::DEFAULT_MAX_HISTORY);
        assert_eq!(session.history.last().unwrap().content, "msg 79");
    }

    #[test]
    fn private_sessions_are_invisible_to_foreigners() {
        let mut manager = AiSessionManager::new();
        let alice = uid(1);
        let bob = uid(99);
        let cube = CubeId::from_u64(5);
        let id = manager.open(
            alice,
            AiSessionScope::Cube(cube),
            ProviderId::new("local"),
            ModelId::new("tiny"),
        );

        assert!(manager.get(id, bob).is_none());
        assert!(manager.get(id, alice).is_some());
        assert!(manager.record(id, bob, AiMessage::user("borrow")).is_err());
        assert_eq!(manager.sessions_of(alice).len(), 1);
        assert!(manager.sessions_of(bob).is_empty());

        // Ownership carries through cube scoping.
        assert!(manager.close(id, bob).is_err());
        manager.close(id, alice).unwrap();
        assert!(manager.get(id, alice).is_none());
        assert!(manager.is_empty());
    }
}
