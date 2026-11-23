//! Client-side application state.
//!
//! Pure, terminal-free state that mirrors what the server told us. The TUI
//! presents this directly; the transport layer only mutates it through
//! [`ClientState::on_network_event`].

use std::collections::VecDeque;
use std::path::PathBuf;

use nexus_core::ids::{CubeId, FileId, UserId, WorkspaceId};

/// Connection lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnectionState {
    #[default]
    Disconnected,
    Connecting,
    Connected,
}

/// Presence status mirror.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MemberStatus {
    #[default]
    Online,
    Busy,
    Away,
    Offline,
}

/// A cube member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberPresence {
    pub user_id: UserId,
    pub name: String,
    pub status: MemberStatus,
}

/// A cube known to the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CubeInfo {
    pub id: CubeId,
    pub name: String,
}

/// The file tree of the current cube.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceState {
    pub workspace: Option<WorkspaceId>,
    /// Flat file list: (id, path within the workspace).
    pub files: Vec<(FileId, PathBuf)>,
}

impl WorkspaceState {
    pub fn apply_listing(&mut self, workspace: WorkspaceId, files: Vec<(FileId, PathBuf)>) {
        self.workspace = Some(workspace);
        self.files = files;
    }

    #[must_use]
    pub fn file_ids(&self) -> impl Iterator<Item = FileId> + '_ {
        self.files.iter().map(|(id, _)| *id)
    }

    /// The path for a file id, if present.
    #[must_use]
    pub fn path_of(&self, file_id: FileId) -> Option<&PathBuf> {
        self.files
            .iter()
            .find(|(id, _)| *id == file_id)
            .map(|(_, p)| p)
    }
}

/// The full client state, consolidated from network events.
#[derive(Debug, Clone, Default)]
pub struct ClientState {
    pub connection: ConnectionState,
    pub user_id: Option<UserId>,
    pub username: Option<String>,
    /// Cubes known to the authenticated user.
    pub cubes: Vec<CubeInfo>,
    /// The cube this client is currently inside.
    pub active_cube: Option<CubeInfo>,
    pub members: Vec<MemberPresence>,
    pub workspace: WorkspaceState,
    /// Limited recent-message queue.
    pub chat: VecDeque<(String, String)>,
    /// The last plugin roster the server reported (Part 11).
    pub plugins: Vec<crate::network::PluginRecordSummary>,
    pub last_error: Option<String>,
}

impl ClientState {
    /// Maximum chat lines retained in state.
    pub const MAX_CHAT: usize = 256;

    /// Applies a single network event, mutating state in place.
    pub fn on_network_event(&mut self, event: &crate::network::NetworkEvent) {
        use crate::network::NetworkEvent as E;
        match event {
            E::Connected => self.connection = ConnectionState::Connected,
            E::Disconnected(reason) => {
                self.connection = ConnectionState::Disconnected;
                self.last_error = reason.clone();
                self.active_cube = None;
            }
            E::Authenticated { user_id, username } => {
                self.user_id = Some(*user_id);
                self.username = Some(username.clone());
            }
            E::AuthFailed(msg) => {
                self.last_error = Some(msg.clone());
                self.connection = ConnectionState::Disconnected;
            }
            E::CubeKnown(id, name) => self.remember_cube(*id, name.clone()),
            E::CubeCreated { id, name } => self.remember_cube(*id, name.clone()),
            E::CubeDeleted(id) => self.cubes.retain(|c| c.id != *id),
            E::Joined { cube, name } => {
                self.active_cube = Some(CubeInfo {
                    id: *cube,
                    name: name.clone(),
                });
            }
            E::Left { .. } => {
                self.active_cube = None;
                self.members.clear();
                self.workspace = WorkspaceState::default();
            }
            E::UsersChanged => {}
            E::WorkspaceListed { workspace } => {
                self.workspace.workspace = Some(*workspace);
            }
            E::FileChanged(_) => {}
            E::Message { from, text, .. } => {
                let name = self
                    .members
                    .iter()
                    .find(|m| m.user_id == *from)
                    .map(|m| m.name.clone())
                    .unwrap_or_else(|| "unknown".to_owned());
                self.chat.push_back((name, text.clone()));
                while self.chat.len() > Self::MAX_CHAT {
                    self.chat.pop_front();
                }
            }
            E::BuildStarted { .. } => {}
            E::BuildFinished { .. } => {}
            E::TerminalOutput { .. } => {}
            E::PluginStatus { records } => {
                self.plugins = records.clone();
            }
            E::Error(msg) => self.last_error = Some(msg.clone()),
        }
    }

    /// Applies events in order and clears the pending queue.
    pub fn drain_events<I>(&mut self, events: I)
    where
        I: IntoIterator<Item = crate::network::NetworkEvent>,
    {
        for event in events {
            self.on_network_event(&event);
        }
    }

    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.connection == ConnectionState::Connected
    }

    /// A status-bar string describing the user.
    #[must_use]
    pub fn user_label(&self) -> String {
        self.username.clone().unwrap_or_else(|| "-".to_owned())
    }

    fn remember_cube(&mut self, id: CubeId, name: String) {
        if !self.cubes.iter().any(|c| c.id == id) {
            self.cubes.push(CubeInfo { id, name });
        }
    }
}
