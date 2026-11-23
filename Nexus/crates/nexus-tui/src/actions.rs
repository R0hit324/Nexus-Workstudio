//! Client action model.
//!
//! [`Action`]s express user intent; the application layer decides whether an
//! action is handled locally, sent to the server, or waits for confirmation.
//! Widgets translate commands into actions through the dispatcher, never
//! touching the network.

use std::path::PathBuf;

use nexus_core::ids::{CubeId, FileId};

/// Class of an action, driving how the client executes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionClass {
    /// Handled entirely in process: no server round trip.
    Local,
    /// Sent to the server; the resulting event updates state.
    Remote,
    /// Requires explicit server confirmation before state commits.
    ServerConfirmed,
}

/// User intent for the client application layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
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
    SendChatMessage(String),
    SwitchTab(usize),
    CommitLocalEdit(FileId),
    RefreshWorkspace,
    ListPlugins,
}

// Convenience alias to keep the enum readable.
use Action as A;

impl Action {
    /// The execution class of this action.
    #[must_use]
    pub fn class(&self) -> ActionClass {
        match self {
            A::SwitchTab(_) | A::CommitLocalEdit(_) | A::RefreshWorkspace | A::ListPlugins => {
                ActionClass::Local
            }
            A::OpenFile(_) | A::CloseFile(_) | A::SaveFile(_) | A::SendChatMessage(_) => {
                ActionClass::Local
            }
            A::Build | A::Run | A::JoinCube(_) | A::LeaveCube | A::CreateCube(_) => {
                ActionClass::Remote
            }
            A::DeleteFile(_) | A::RenameFile(_, _) | A::CreateFile(_) => {
                ActionClass::ServerConfirmed
            }
        }
    }

    /// `true` when the action mutates only client-side state.
    #[must_use]
    pub fn is_local(&self) -> bool {
        matches!(self.class(), ActionClass::Local)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::ids::CubeId;

    #[test]
    fn action_classes_are_correct() {
        assert!(Action::SwitchTab(0).is_local());
        assert!(Action::CommitLocalEdit(FileId::new()).is_local());
        assert_eq!(Action::Build.class(), ActionClass::Remote);
        assert_eq!(Action::JoinCube(CubeId::new()).class(), ActionClass::Remote);
        assert_eq!(
            Action::DeleteFile(FileId::new()).class(),
            ActionClass::ServerConfirmed
        );
    }
}
