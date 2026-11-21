//! Server-side session lifecycle: [`SessionState`] transitions and the
//! authoritative [`SessionManager`].

pub mod manager;
pub mod session;
pub mod state;

pub use manager::SessionManager;
pub use session::ServerSession;
pub use state::SessionState;
