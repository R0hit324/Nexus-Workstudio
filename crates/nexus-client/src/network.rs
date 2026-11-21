//! Network event model.
//!
//! The session service translates protocol messages into these typed events.
//! The TUI and the headless client consume them symmetrically, so UI code
//! never touches transport details.

use nexus_core::ids::{CubeId, FileId, UserId, WorkspaceId};

/// One row of the plugin roster reported to the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRecordSummary {
    pub name: String,
    pub version: String,
    pub state: String,
    pub trust: String,
    pub error: Option<String>,
}

/// A derived, UI-relevant network event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkEvent {
    /// Connected and handshake completed.
    Connected,
    /// Lost the connection; carries an optional reason.
    Disconnected(Option<String>),
    /// Signed in as `username`.
    Authenticated {
        user_id: UserId,
        username: String,
    },
    AuthFailed(String),
    /// A cube was listed or created.
    CubeKnown(CubeId, String),
    CubeCreated {
        id: CubeId,
        name: String,
    },
    CubeDeleted(CubeId),
    Joined {
        cube: CubeId,
        name: String,
    },
    Left {
        cube: CubeId,
    },
    /// Membership or presence of the current cube changed.
    UsersChanged,
    /// The workspace for the current cube was (re)listed.
    WorkspaceListed {
        workspace: WorkspaceId,
    },
    /// A file's remote content changed.
    FileChanged(FileId),
    /// A chat message arrived for the current cube.
    Message {
        cube: CubeId,
        from: UserId,
        text: String,
    },
    BuildStarted {
        command: String,
    },
    BuildFinished {
        success: bool,
        diagnostics: usize,
    },
    /// A line of terminal output. `stream` 0 = stdout, 1 = stderr.
    TerminalOutput {
        stream: u8,
        line: String,
    },
    /// The plugin roster reply to a UI request (Part 11).
    PluginStatus {
        records: Vec<PluginRecordSummary>,
    },
    /// A generic error surfaced to the user.
    Error(String),
}
