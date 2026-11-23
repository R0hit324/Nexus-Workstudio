use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use nexus_core::error::Error;
use nexus_core::ids::UserId;
use tokio::sync::Mutex;

use crate::users::{User, UserRegistry};

/// Credentials presented during the authentication phase of a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticationRequest {
    pub username: String,
    /// Used for audit trails and rate limiting.
    pub peer: SocketAddr,
}

/// The verified identity produced by an [`Authenticator`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticationIdentity {
    pub user_id: UserId,
    pub username: String,
    pub display_name: String,
}

impl From<User> for AuthenticationIdentity {
    fn from(user: User) -> Self {
        Self {
            user_id: user.id(),
            username: user.username().to_owned(),
            display_name: user.display_name().to_owned(),
        }
    }
}

/// Strategy that decides whether a client-provided identity is acceptable.
///
/// Part 3 ships [`TrustAuthenticator`]; later parts plug in credential
/// verification (tokens, session keys, SSO) behind this same seam.
#[async_trait::async_trait]
pub trait Authenticator: Send + Sync {
    async fn authenticate(
        &self,
        request: AuthenticationRequest,
    ) -> nexus_core::Result<AuthenticationIdentity>;
}

/// Fixed-window rate limiter used to harden the pre-authentication path.
///
/// Each peer gets `burst` attempts per `window`, tracked independently so one
/// abusive peer cannot exhaust the budget of another.
struct AuthRateLimiter {
    state: Mutex<std::collections::HashMap<SocketAddr, (usize, std::time::Instant)>>,
    burst: usize,
    window: Duration,
}

impl AuthRateLimiter {
    fn new(burst: usize, window: Duration) -> Self {
        Self {
            state: Mutex::new(std::collections::HashMap::new()),
            burst: burst.max(1),
            window,
        }
    }

    /// Returns `false` when the peer has exhausted its allowance.
    async fn allow(&self, peer: SocketAddr) -> bool {
        let mut state = self.state.lock().await;
        let now = std::time::Instant::now();
        let entry = state.get(&peer).copied();
        if let Some((count, window_start)) = entry {
            if now.duration_since(window_start) > self.window {
                state.insert(peer, (1, now));
                return true;
            }
            if count >= self.burst {
                return false;
            }
            state.insert(peer, (count + 1, window_start));
            true
        } else {
            state.insert(peer, (1, now));
            true
        }
    }
}

/// The Part 3 authenticator. Trusts any syntactically valid username and
/// registers it on first sight.
///
/// Security posture for LAN-first: the username is the identity, no secret is
/// exchanged, and rate limiting bounds brute-force/DoS on the handshake path.
pub struct TrustAuthenticator {
    users: Arc<UserRegistry>,
    limiter: AuthRateLimiter,
}

impl TrustAuthenticator {
    pub fn new(users: Arc<UserRegistry>) -> Self {
        Self::with_limits(users, 10, Duration::from_secs(60))
    }

    pub fn with_limits(users: Arc<UserRegistry>, burst: usize, window: Duration) -> Self {
        Self {
            users,
            limiter: AuthRateLimiter::new(burst, window),
        }
    }
}

#[async_trait::async_trait]
impl Authenticator for TrustAuthenticator {
    async fn authenticate(
        &self,
        request: AuthenticationRequest,
    ) -> nexus_core::Result<AuthenticationIdentity> {
        if !self.limiter.allow(request.peer).await {
            return Err(Error::rate_limited(
                "too many authentication attempts, retry shortly",
            ));
        }
        let user = self
            .users
            .get_or_register(&request.username, &request.username)?;
        Ok(AuthenticationIdentity::from(user))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn trust_authenticates_any_valid_username() {
        let users = Arc::new(UserRegistry::new());
        let auth = TrustAuthenticator::new(users.clone());
        let request = AuthenticationRequest {
            username: "alice".into(),
            peer: "127.0.0.1:4000".parse().unwrap(),
        };
        let identity = auth.authenticate(request).await.unwrap();
        assert_eq!(identity.username, "alice");
        assert_eq!(users.len(), 1);
    }

    #[tokio::test]
    async fn invalid_usernames_rejected() {
        let users = Arc::new(UserRegistry::new());
        let auth = TrustAuthenticator::new(users.clone());
        let request = AuthenticationRequest {
            username: "bad name".into(),
            peer: "127.0.0.1:4000".parse().unwrap(),
        };
        let err = auth.authenticate(request).await.unwrap_err();
        assert_eq!(err.code(), nexus_core::error::ErrorCode::InvalidRequest);
    }

    #[tokio::test]
    async fn rate_limiter_bounds_attempts_per_peer() {
        let users = Arc::new(UserRegistry::new());
        let auth = TrustAuthenticator::with_limits(users, 3, Duration::from_secs(60));
        let peer: SocketAddr = "127.0.0.1:4001".parse().unwrap();
        for _ in 0..3 {
            let request = AuthenticationRequest {
                username: "alice".into(),
                peer,
            };
            assert!(auth.authenticate(request).await.is_ok());
        }
        let exhausted = AuthenticationRequest {
            username: "alice".into(),
            peer,
        };
        let err = auth.authenticate(exhausted).await.unwrap_err();
        assert_eq!(err.code(), nexus_core::error::ErrorCode::RateLimited);
    }

    #[tokio::test]
    async fn other_peers_are_not_penalized() {
        let users = Arc::new(UserRegistry::new());
        let auth = TrustAuthenticator::with_limits(users, 1, Duration::from_secs(60));
        let first_peer: SocketAddr = "127.0.0.1:4100".parse().unwrap();
        let second_peer: SocketAddr = "127.0.0.1:4101".parse().unwrap();
        for peer in [first_peer, second_peer] {
            let request = AuthenticationRequest {
                username: "alice".into(),
                peer,
            };
            assert!(auth.authenticate(request).await.is_ok());
        }
    }
}
