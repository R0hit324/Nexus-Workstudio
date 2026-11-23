//! Client application layer: state and actions.

pub mod actions;
pub mod state;

pub use actions::ClientAction;
pub use state::{
    ClientState, ConnectionState, CubeInfo, MemberPresence, MemberStatus, WorkspaceState,
};
