//! Client-side actions the TUI and headless automation can issue.
//!
//! These mirror the wire surface of the client. The TUI's presentation-level
//! actions (in `nexus-tui`) translate into these before reaching service code.

use std::path::PathBuf;

use nexus_core::ids::{CubeId, FileId};

/// A concrete operation the client can execute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientAction {
    OpenFile(FileId),
    SaveFile(FileId),
    CloseFile(FileId),
    CreateFile(PathBuf),
    DeleteFile(FileId),
    RenameFile(FileId, PathBuf),
    JoinCube(CubeId),
    LeaveCube,
    CreateCube(String),
    Build,
    Run,
    SendChat(String),
}

impl ClientAction {
    /// Whether the server must confirm this action before state commits.
    #[must_use]
    pub const fn is_server_confirmed(&self) -> bool {
        matches!(
            self,
            Self::CreateFile(_) | Self::DeleteFile(_) | Self::RenameFile(_, _)
        )
    }
}
