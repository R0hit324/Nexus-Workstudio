//! Session service: the single facade the UI drives.
//!
//! In later parts this wraps the real transport (`crate::client::Client`). For
//! Part 2 the service is a faithful *offline simulation*: every operation
//! updates local [`ClientState`] and emits the matching [`NetworkEvent`]
//! through [`EventSink`] exactly as the transport would. The UI therefore
//! binds once to this service; swapping in a live transport later changes
//! nothing else.

use std::path::PathBuf;

use nexus_core::ids::{CubeId, FileId, UserId, WorkspaceId};

use super::app::state::{ClientState, CubeInfo, MemberPresence, MemberStatus};
use super::events::EventSink;
use super::network::{NetworkEvent, PluginRecordSummary};
use super::workspace::WorkspaceService;

/// Drives authentication, cube memberships, workspace, build, and chat for
/// one client session.
#[derive(Debug)]
pub struct SessionService {
    /// Consolidated client state (mirrors the server).
    pub state: ClientState,
    /// Workspace mirror + content cache.
    pub workspace: WorkspaceService,
    /// Pending events consumed by the application loop.
    pub sink: EventSink,
    pub address: String,
    /// The local user once authenticated.
    pub me: Option<MemberPresence>,
}

impl SessionService {
    /// Creates a disconnected session targeting `address`.
    #[must_use]
    pub fn new(address: impl Into<String>) -> Self {
        Self {
            state: ClientState::default(),
            workspace: WorkspaceService::new(),
            sink: EventSink::new(),
            address: address.into(),
            me: None,
        }
    }

    /// Opens a connection and authenticates as `username`.
    ///
    /// Net effect: `Connected`, `Authenticated`, then `Error` on failure.
    pub fn connect(&mut self, username: impl Into<String>) {
        self.state.connection = super::app::state::ConnectionState::Connecting;
        self.sink.push(NetworkEvent::Connected);
        let username = username.into();
        let user_id = UserId::new();
        self.me = Some(MemberPresence {
            user_id,
            name: username.clone(),
            status: MemberStatus::Online,
        });
        self.sink
            .push(NetworkEvent::Authenticated { user_id, username });
    }

    pub fn disconnect(&mut self) {
        self.sink.push(NetworkEvent::Disconnected(None));
    }

    /// Syncs the known cube list from the server.
    pub fn list_cubes(&mut self, cubes: &[CubeInfo]) {
        for cube in cubes {
            self.sink
                .push(NetworkEvent::CubeKnown(cube.id, cube.name.clone()));
        }
    }

    /// Creates a cube, returning its id (server-generated in the future).
    pub fn create_cube(&mut self, name: impl Into<String>) -> CubeId {
        let id = CubeId::new();
        self.sink.push(NetworkEvent::CubeCreated {
            id,
            name: name.into(),
        });
        id
    }

    /// Joins a cube, registering that membership with the server.
    pub fn join_cube(
        &mut self,
        cube: CubeId,
        name: impl Into<String>,
        workspace: WorkspaceId,
        files: Vec<(FileId, PathBuf)>,
    ) {
        let name = name.into();
        self.sink.push(NetworkEvent::Joined {
            cube,
            name: name.clone(),
        });
        self.workspace.set_listing(workspace, files);
        self.sink.push(NetworkEvent::WorkspaceListed { workspace });
        self.sink.push(NetworkEvent::UsersChanged);
    }

    pub fn leave_cube(&mut self) {
        if let Some(cube) = self.state.active_cube.as_ref().map(|c| c.id) {
            self.sink.push(NetworkEvent::Left { cube });
        }
    }

    /// Replaces the member/presence list for the active cube.
    pub fn set_presence(&mut self, members: Vec<MemberPresence>) {
        self.state.members = members;
        self.sink.push(NetworkEvent::UsersChanged);
    }

    /// Caches opened file content (no network round trip to open).
    pub fn open_file(&mut self, file_id: FileId, path: PathBuf, content: String) {
        self.workspace.cache_content(file_id, path.clone(), content);
        self.sink.push(NetworkEvent::FileChanged(file_id));
    }

    /// Marks a file saved; emits no event (persist is fire-and-forget here).
    pub fn save_file(&mut self, file_id: FileId) {
        self.workspace.mark_saved(file_id);
    }

    /// Starts a build for the active cube.
    pub fn build(&mut self, command: impl Into<String>) {
        self.sink.push(NetworkEvent::BuildStarted {
            command: command.into(),
        });
    }

    /// Reports a finished build.
    pub fn finish_build(&mut self, success: bool, diagnostics: usize) {
        self.sink.push(NetworkEvent::BuildFinished {
            success,
            diagnostics,
        });
    }

    /// Starts a run.
    pub fn run(&mut self, command: impl Into<String>) {
        self.sink.push(NetworkEvent::BuildStarted {
            command: command.into(),
        });
    }

    /// Sends a chat message as the local user.
    pub fn send_chat(&mut self, text: impl Into<String>) {
        let text = text.into();
        if let (Some(me), Some(cube)) = (&self.me, self.state.active_cube.as_ref()) {
            self.sink.push(NetworkEvent::Message {
                cube: cube.id,
                from: me.user_id,
                text,
            });
        }
    }

    /// Feeds a line of terminal output into the sink.
    pub fn terminal_output(&mut self, stream: u8, line: impl Into<String>) {
        self.sink.push(NetworkEvent::TerminalOutput {
            stream,
            line: line.into(),
        });
    }

    /// Requests the plugin roster (Part 11). The simulated server reports the
    /// two built-in plugins, which is exactly what a live server serves when
    /// no external manifests are installed.
    pub fn plugin_status(&mut self) {
        let records = vec![
            PluginRecordSummary {
                name: String::from("Git Integration"),
                version: String::from("1.0.0"),
                state: String::from("enabled"),
                trust: String::from("trusted"),
                error: None,
            },
            PluginRecordSummary {
                name: String::from("Rust Support"),
                version: String::from("1.0.0"),
                state: String::from("enabled"),
                trust: String::from("trusted"),
                error: None,
            },
        ];
        self.sink.push(NetworkEvent::PluginStatus { records });
    }

    /// Drains pending events, updating state as the server would have.
    #[must_use]
    pub fn drain(&mut self) -> Vec<NetworkEvent> {
        self.sink.drain(&mut self.state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> Vec<(FileId, PathBuf)> {
        vec![
            (FileId::from_u64(1), PathBuf::from("src/main.rs")),
            (FileId::from_u64(2), PathBuf::from("README.md")),
        ]
    }

    #[test]
    fn connect_authenticates_and_updates_state() {
        let mut session = SessionService::new("ws://localhost:9000");
        session.connect("alice");
        let events = session.drain();
        assert!(matches!(
            events
                .iter()
                .find(|e| matches!(e, NetworkEvent::Authenticated { .. })),
            Some(NetworkEvent::Authenticated { .. })
        ));
        assert_eq!(session.state.username.as_deref(), Some("alice"));
        assert!(session.state.is_connected());
    }

    #[test]
    fn join_populates_workspace_and_active_cube() {
        let mut session = SessionService::new("ws://x");
        session.connect("bob");
        let _ = session.drain();
        let cube = CubeId::from_u64(7);
        session.join_cube(cube, "api", WorkspaceId::from_u64(1), files());
        let events = session.drain();
        assert!(events
            .iter()
            .any(|e| matches!(e, NetworkEvent::Joined { .. })));
        assert_eq!(session.state.active_cube.as_ref().unwrap().id, cube);
        assert_eq!(session.workspace.state.files.len(), 2);
        assert_eq!(
            session.workspace.path_of(FileId::from_u64(1)).unwrap(),
            &PathBuf::from("src/main.rs")
        );
    }

    #[test]
    fn open_save_mark_dirty_flow() {
        let mut session = SessionService::new("ws://x");
        session.connect("c");
        let _ = session.drain();
        let id = FileId::from_u64(1);
        session
            .workspace
            .set_listing(WorkspaceId::from_u64(1), files());
        session.open_file(id, PathBuf::from("src/main.rs"), "fn x() {}".into());
        assert_eq!(session.workspace.cached(id), Some("fn x() {}"));
        session.workspace.mark_dirty(id);
        assert!(session.workspace.is_dirty(id));
        session.save_file(id);
        assert!(!session.workspace.is_dirty(id));
    }

    #[test]
    fn chat_only_sends_inside_a_cube() {
        let mut session = SessionService::new("ws://x");
        session.connect("dave");
        let _ = session.drain();
        session.send_chat("hi"); // not in a cube yet: no event
        assert!(session.sink.is_empty());
        session.join_cube(CubeId::from_u64(1), "c", WorkspaceId::from_u64(1), files());
        let _ = session.drain();
        session.send_chat("hi");
        assert!(!session.sink.is_empty());
        let events = session.drain();
        assert!(events
            .iter()
            .any(|e| matches!(e, NetworkEvent::Message { .. })));
    }

    #[test]
    fn build_lifecycle() {
        let mut session = SessionService::new("ws://x");
        session.connect("e");
        let _ = session.drain();
        session.build("cargo build");
        assert!(matches!(
            session.drain().first(),
            Some(NetworkEvent::BuildStarted { .. })
        ));
        session.finish_build(true, 0);
        assert!(matches!(
            session.drain().first(),
            Some(NetworkEvent::BuildFinished { success: true, .. })
        ));
    }

    #[test]
    fn plugin_status_reports_builtin_roster() {
        let mut session = SessionService::new("ws://x");
        session.connect("f");
        let _ = session.drain();
        session.plugin_status();
        let events = session.drain();
        let Some(NetworkEvent::PluginStatus { records }) = events
            .iter()
            .find(|e| matches!(e, NetworkEvent::PluginStatus { .. }))
        else {
            panic!("expected PluginStatus event");
        };
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].name, "Git Integration");
        assert_eq!(records[1].state, "enabled");
        assert!(records.iter().all(|r| r.error.is_none()));
        // The roster is mirrored into client state for the status bar.
        assert_eq!(session.state.plugins.len(), 2);
    }
}
