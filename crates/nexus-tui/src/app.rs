//! The application: state ownership, command dispatch, event flow, rendering.
//!
//! ```text
//! crossterm events → InputTranslator → UiEvent
//!   → App::handle_event (mode-aware)
//!     → Keymap resolve → Command → App::dispatch_command
//!     → App::apply_action → SessionService (network) or local editor
//!     → SessionService::drain → NetworkEvent → App::on_network_event
//!   → App::render (ratatui, terminal-only layer)
//! ```
//!
//! All domain logic is testable headlessly; the terminal surface is confined
//! to `render`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use nexus_client::app::state::{ConnectionState, MemberStatus};
use nexus_client::network::NetworkEvent;
use nexus_client::SessionService;
use nexus_core::ids::{CubeId, FileId, WorkspaceId};
use ratatui::Frame;

use crate::actions::Action;
use crate::chat::{ChatLineKind, ChatState};
use crate::commands::{Command, CommandRegistry};
use crate::dialogs::DialogQueue;
use crate::editor::{Cursor, Document, RemoteCursorStore};
use crate::explorer::Explorer;
use crate::input::{Key, MouseButton, MouseEvent, MouseKind, UiEvent};
use crate::keymap::{Keymap, Mode};
use crate::layout::{PanelId, PanelRuntime};
use crate::notifications::{NotificationCenter, NotificationKind};
use crate::palette::CommandPalette;
use crate::persistence::{LineNumbersMode, LocalPreferences};
use crate::status::UiSnapshot;
use crate::theme::{ColorSpec, StyleKey, StyleSpec, Theme};
use crate::MAX_OPEN_DOCUMENTS;

/// Which prompt is currently being edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(dead_code)]
enum ActiveInput {
    #[default]
    None,
    CommandLine,
    InputDialog,
}

/// All open editor documents plus tab/split structure.
#[derive(Debug, Default)]
pub struct EditorState {
    pub docs: HashMap<FileId, Document>,
    pub tabs: crate::tabs::Tabs,
    pub splits: crate::splits::SplitTree,
    pub remote_cursors: RemoteCursorStore,
    pub line_numbers: LineNumbersMode,
}

impl EditorState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            docs: HashMap::new(),
            tabs: crate::tabs::Tabs::default(),
            splits: crate::splits::SplitTree::single(),
            remote_cursors: RemoteCursorStore::default(),
            line_numbers: LineNumbersMode::Relative,
        }
    }

    /// Opens/caches a document and shows it in a tab. Evicts a document beyond
    /// `MAX_OPEN_DOCUMENTS`, preferring those not open in any tab.
    pub fn open(&mut self, file_id: FileId, path: PathBuf, text: &str, large: bool) {
        if !self.docs.contains_key(&file_id) {
            self.docs
                .insert(file_id, Document::open(file_id, path.clone(), text, large));
        }
        while self.docs.len() > MAX_OPEN_DOCUMENTS {
            let tabbed: HashSet<FileId> = self.tabs.file_ids().collect();
            let droppable = self
                .docs
                .keys()
                .find(|id| !tabbed.contains(id))
                .copied()
                .unwrap_or_else(|| file_id);
            let Some(removed) = self.docs.remove(&droppable) else {
                break;
            };
            let _ = removed;
        }
        let label = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("untitled")
            .to_owned();
        self.tabs.open(file_id, label);
    }

    /// Closes the active tab and drops its document if not open elsewhere.
    pub fn close_active(&mut self) -> Option<FileId> {
        let closed = self.tabs.close_active()?;
        if !self.tabs.file_ids().any(|t| t == closed) {
            self.docs.remove(&closed);
        }
        Some(closed)
    }

    #[must_use]
    pub fn active_doc_id(&self) -> Option<FileId> {
        self.tabs.active_tab().map(|t| t.file_id)
    }

    #[must_use]
    pub fn active_doc(&self) -> Option<&Document> {
        self.active_doc_id().and_then(|id| self.docs.get(&id))
    }

    #[must_use]
    pub fn active_doc_mut(&mut self) -> Option<&mut Document> {
        let id = self.active_doc_id()?;
        self.docs.get_mut(&id)
    }
}

/// The whole TUI application.
pub struct App {
    pub session: SessionService,
    pub keymap: Keymap,
    pub registry: CommandRegistry,
    pub mode: Mode,
    pub prefs: LocalPreferences,
    pub theme: Theme,
    pub editor: EditorState,
    pub explorer: Explorer,
    pub chat: ChatState,
    pub users: crate::users::UsersPanelState,
    pub terminal: crate::terminal_panel::TerminalPanelState,
    pub build: crate::build_panel::BuildState,
    pub notifications: NotificationCenter,
    pub dialogs: DialogQueue,
    pub palette: Option<CommandPalette>,
    pub layout: PanelRuntime,
    pub snapshot: UiSnapshot,
    pub cols: u16,
    pub rows: u16,
    pub quit: bool,
    active_input: ActiveInput,
    pub command_line: String,
    pub dialog_text: String,
}

impl App {
    /// Builds an application around a session service and default prefs.
    #[must_use]
    pub fn new(session: SessionService) -> Self {
        let prefs = LocalPreferences::load(&LocalPreferences::default_path());
        let mut keymap = Keymap::default();
        let user_specs = prefs
            .bindings
            .iter()
            .map(|b| format!("normal:{}={}", b.key, b.command))
            .collect::<Vec<_>>();
        let _ = keymap.apply_specs(&user_specs);

        Self {
            session,
            keymap,
            registry: CommandRegistry::default(),
            mode: Mode::Normal,
            prefs,
            theme: crate::theme::default_theme(),
            editor: EditorState::new(),
            explorer: Explorer::new(PathBuf::from(".")),
            chat: ChatState::new(),
            users: crate::users::UsersPanelState::default(),
            terminal: crate::terminal_panel::TerminalPanelState::new(),
            build: crate::build_panel::BuildState::new(),
            notifications: NotificationCenter::default(),
            dialogs: DialogQueue::new(),
            palette: None,
            layout: PanelRuntime::default(),
            snapshot: UiSnapshot::default(),
            cols: 120,
            rows: 40,
            quit: false,
            active_input: ActiveInput::None,
            command_line: String::new(),
            dialog_text: String::new(),
        }
    }

    /// Central event handling: all input flows through here.
    pub fn handle_event(&mut self, event: UiEvent) {
        match event {
            UiEvent::Key(key) => self.on_key(key),
            UiEvent::Mouse(mouse) => self.on_mouse(mouse),
            UiEvent::Resize(cols, rows) => self.resize(cols, rows),
            UiEvent::Tick => self.tick(),
            UiEvent::Network(evt) => self.on_network_event(evt),
            UiEvent::Command(command) => self.dispatch_command(command),
            UiEvent::Action(action) => self.apply_action(action),
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.cols = cols.max(40);
        self.rows = rows.max(8);
        self.layout.layout(self.cols, self.rows);
        if self.layout.focus.is_none() {
            self.layout.set_focus(PanelId::Editor);
        }
    }

    fn on_key(&mut self, key: Key) {
        // Overlays capture input first.
        if self.palette.is_some() {
            let mut palette = self.palette.take().expect("checked");
            let mut close = false;
            match key {
                Key::Esc | Key::Ctrl('c') => close = true,
                Key::Enter => {
                    close = true;
                    if let Some(command) = palette.selected() {
                        self.dispatch_command(command);
                    }
                }
                Key::Up => palette.move_selection(-1),
                Key::Down => palette.move_selection(1),
                Key::PageUp => palette.move_selection(-5),
                Key::PageDown => palette.move_selection(5),
                Key::Backspace => {
                    palette.query.pop();
                    palette.refresh(&self.registry);
                }
                Key::Char(c) => {
                    palette.query.push(c);
                    palette.refresh(&self.registry);
                }
                _ => {}
            }
            if !close {
                self.palette = Some(palette);
            }
            return;
        }

        if !self.dialogs.is_empty() {
            self.dialog_key(key);
            return;
        }

        match self.mode {
            Mode::Normal => self.normal_mode_key(key),
            Mode::Insert => self.insert_mode_key(key),
            Mode::Search => self.search_mode_key(key),
            Mode::Command => self.command_mode_key(key),
        }
    }

    fn dialog_key(&mut self, key: Key) {
        use crate::dialogs::DialogKind;
        let Some(kind) = self.dialogs.active().map(|d| d.kind) else {
            return;
        };
        match (kind, key) {
            (_, Key::Esc) => {
                self.dialogs.cancel();
                self.active_input = ActiveInput::None;
            }
            (DialogKind::Error | DialogKind::Info, Key::Enter) => {
                self.dialogs.cancel();
            }
            (DialogKind::Confirm, Key::Enter) => {
                if let Some(action) = self.dialogs.confirm_top() {
                    self.apply_action(action);
                }
            }
            (DialogKind::Input, Key::Enter) => {
                if let Some(action) = self.dialogs.confirm_top() {
                    self.dialog_text = String::new();
                    self.active_input = ActiveInput::None;
                    self.apply_action(action);
                }
            }
            (DialogKind::Input, Key::Char(c)) => self.dialog_text.push(c),
            (DialogKind::Input, Key::Backspace) => {
                self.dialog_text.pop();
            }
            _ => {}
        }
    }

    fn normal_mode_key(&mut self, key: Key) {
        // Plain mode switches that the keymap does not cover.
        if matches!(key, Key::Char('i') | Key::Char('a') | Key::Char('o')) {
            self.enter_insert();
            return;
        }
        let Some(command) = self.keymap.resolve(Mode::Normal, key) else {
            return;
        };
        self.dispatch_command(command);
    }

    fn insert_mode_key(&mut self, key: Key) {
        match key {
            Key::Esc => self.mode = Mode::Normal,
            Key::Char(c) => self.type_text(&c.to_string()),
            Key::Enter => self.type_text("\n"),
            Key::Tab => self.type_text("\t"),
            Key::Backspace => {
                if let Some(doc) = self.editor.active_doc_mut() {
                    doc.delete_backward();
                }
            }
            Key::Delete => {
                if let Some(doc) = self.editor.active_doc_mut() {
                    doc.delete_forward();
                }
            }
            Key::Left => self.move_cursor(-1, 0),
            Key::Right => self.move_cursor(1, 0),
            Key::Up => self.move_cursor(0, -1),
            Key::Down => self.move_cursor(0, 1),
            Key::Home => {
                if let Some(doc) = self.editor.active_doc_mut() {
                    doc.cursor.to_line_start();
                }
            }
            Key::End => {
                if let Some(doc) = self.editor.active_doc_mut() {
                    doc.cursor.to_line_end(&doc.buffer);
                }
            }
            _ => {}
        }
    }

    fn search_mode_key(&mut self, key: Key) {
        match key {
            Key::Esc => {
                self.mode = Mode::Normal;
                if let Some(doc) = self.editor.active_doc_mut() {
                    doc.search.set_query(&doc.buffer, "");
                }
            }
            Key::Enter => {
                self.mode = Mode::Normal;
                self.jump_to_match(0);
            }
            Key::Backspace => {
                if let Some(doc) = self.editor.active_doc_mut() {
                    doc.search.query.pop();
                    let q = doc.search.query.clone();
                    doc.search.set_query(&doc.buffer, &q);
                }
            }
            Key::Char(c) => {
                if let Some(doc) = self.editor.active_doc_mut() {
                    doc.search.query.push(c);
                    let q = doc.search.query.clone();
                    doc.search.set_query(&doc.buffer, &q);
                }
            }
            _ => {}
        }
    }

    fn command_mode_key(&mut self, key: Key) {
        match key {
            Key::Esc => {
                self.mode = Mode::Normal;
                self.command_line.clear();
            }
            Key::Enter => {
                let line = std::mem::take(&mut self.command_line);
                self.mode = Mode::Normal;
                if let Some(command) = parse_command_line(&line, &self.registry) {
                    self.dispatch_command(command);
                }
            }
            Key::Backspace => {
                self.command_line.pop();
            }
            Key::Char(c) => self.command_line.push(c),
            _ => {}
        }
    }

    fn on_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseKind::Click if mouse.button == MouseButton::Left => {
                if let Some(panel) = self.panel_at(mouse.col, mouse.row) {
                    self.layout.set_focus(panel);
                }
            }
            MouseKind::ScrollUp => self.scroll_focused(3),
            MouseKind::ScrollDown => self.scroll_focused(-3),
            _ => {}
        }
    }

    fn scroll_focused(&mut self, delta: isize) {
        match self.layout.focus {
            Some(PanelId::Terminal) => self.terminal.scroll(delta),
            Some(PanelId::Chat) => self.chat.scroll(delta),
            _ => {
                if let Some(doc) = self.editor.active_doc_mut() {
                    let line_delta = if delta > 0 { -1 } else { 1 };
                    doc.cursor.move_line(&doc.buffer, line_delta);
                }
            }
        }
    }

    /// Which panel a terminal cell belongs to.
    #[must_use]
    pub fn panel_at(&self, col: u16, row: u16) -> Option<PanelId> {
        self.layout
            .rects
            .iter()
            .find(|(_, r)| col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height)
            .map(|(id, _)| *id)
    }

    /// Per-frame work: drain session events, expire notifications.
    pub fn tick(&mut self) {
        let events = self.session.drain();
        for event in events {
            self.on_network_event(event);
        }
        self.notifications.sweep();
    }

    /// Applies a single server-derived network event to the UI.
    pub fn on_network_event(&mut self, event: NetworkEvent) {
        match event {
            NetworkEvent::Connected => {
                self.notifications
                    .push(NotificationKind::Success, "Connected to server");
            }
            NetworkEvent::Disconnected(reason) => {
                self.notifications.push_sticky(
                    NotificationKind::Error,
                    format!("Connection lost: {reason:?}"),
                );
            }
            NetworkEvent::Authenticated { username, .. } => {
                self.notifications
                    .push(NotificationKind::Info, format!("Signed in as {username}"));
            }
            NetworkEvent::AuthFailed(msg) => {
                self.notifications.push(
                    NotificationKind::Error,
                    format!("Authentication failed: {msg}"),
                );
            }
            NetworkEvent::Joined { name, .. } => {
                self.notifications
                    .push(NotificationKind::Success, format!("Joined cube {name}"));
            }
            NetworkEvent::Left { .. } => {
                self.chat.lines.clear();
                self.terminal.clear();
                self.build.clear();
                self.notifications
                    .push(NotificationKind::Info, "Left the cube");
            }
            NetworkEvent::UsersChanged => self.sync_users(),
            NetworkEvent::WorkspaceListed { .. } | NetworkEvent::FileChanged(_) => {}
            NetworkEvent::Message { from, text, .. } => {
                let from_name = self
                    .users
                    .members()
                    .iter()
                    .find(|m| m.user_id == from)
                    .map(|m| m.name.clone())
                    .unwrap_or_else(|| "unknown".to_owned());
                self.chat.push(from_name, text, ChatLineKind::Message);
            }
            NetworkEvent::BuildStarted { command } => self.build.start(command),
            NetworkEvent::BuildFinished { success, .. } => {
                use crate::build_panel::BuildOutcome;
                self.build.finish(if success {
                    BuildOutcome::Success
                } else {
                    BuildOutcome::Failed
                });
                self.notifications.push(
                    if success {
                        NotificationKind::Success
                    } else {
                        NotificationKind::Error
                    },
                    if success {
                        "Build succeeded"
                    } else {
                        "Build failed"
                    },
                );
            }
            NetworkEvent::TerminalOutput { stream, line } => {
                self.terminal.push(line, stream == 1);
            }
            NetworkEvent::Error(msg) => {
                self.notifications.push(NotificationKind::Error, msg);
            }
            NetworkEvent::PluginStatus { records } => {
                if records.is_empty() {
                    self.notifications
                        .push(NotificationKind::Info, "Plugins: none installed");
                } else {
                    let summary = records
                        .iter()
                        .map(|r| match &r.error {
                            Some(error) => {
                                format!("{} {} [{}] ({error})", r.name, r.version, r.state)
                            }
                            None => format!("{} {} [{}]", r.name, r.version, r.state),
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.notifications
                        .push(NotificationKind::Info, format!("Plugins: {summary}"));
                }
            }
            NetworkEvent::CubeKnown(_, _)
            | NetworkEvent::CubeCreated { .. }
            | NetworkEvent::CubeDeleted(_) => {}
        }
    }

    /// Mirrors client presence state into the users panel.
    fn sync_users(&mut self) {
        use crate::users::{UserPresence, UserStatus};
        self.users = crate::users::UsersPanelState::default();
        for member in &self.session.state.members {
            let status = match member.status {
                MemberStatus::Online => UserStatus::Online,
                MemberStatus::Busy => UserStatus::Busy,
                MemberStatus::Away => UserStatus::Away,
                MemberStatus::Offline => UserStatus::Offline,
            };
            self.users.upsert(UserPresence {
                user_id: member.user_id,
                name: member.name.clone(),
                status,
                idle_seconds: 0,
            });
        }
    }

    fn enter_insert(&mut self) {
        self.mode = Mode::Insert;
        if let Some(doc) = self.editor.active_doc_mut() {
            doc.commit_undo();
        }
    }

    fn type_text(&mut self, text: &str) {
        if let Some(doc) = self.editor.active_doc_mut() {
            doc.insert_at_cursor(text);
            self.session.workspace.mark_dirty(doc.file_id);
        }
    }

    fn move_cursor(&mut self, dx: isize, dy: isize) {
        if let Some(doc) = self.editor.active_doc_mut() {
            let mut cursor = Cursor::new(doc.cursor.offset);
            if dx != 0 {
                if dx > 0 {
                    cursor.move_right(&doc.buffer);
                } else {
                    cursor.move_left(&doc.buffer);
                }
            }
            if dy != 0 {
                cursor.move_line(&doc.buffer, dy);
            }
            doc.cursor = cursor;
        }
    }

    fn jump_to_match(&mut self, delta: isize) {
        if let Some(doc) = self.editor.active_doc_mut() {
            if let Some((start, _)) = doc.search.cycle(delta) {
                doc.cursor.offset = start;
            }
        }
    }

    /// Runs a command in the current mode/context.
    pub fn dispatch_command(&mut self, command: Command) {
        use Command as C;
        match command {
            C::ModeInsert => self.enter_insert(),
            C::ModeNormal => {
                self.mode = Mode::Normal;
                if let Some(doc) = self.editor.active_doc_mut() {
                    doc.commit_undo();
                }
            }
            C::Enter => {
                if self.mode == Mode::Search {
                    self.jump_to_match(0);
                }
            }
            C::Search => {
                self.mode = Mode::Search;
                if let Some(doc) = self.editor.active_doc_mut() {
                    let q = doc.search.query.clone();
                    doc.search.set_query(&doc.buffer, &q);
                }
            }
            C::SearchNext => self.jump_to_match(1),
            C::SearchPrev => self.jump_to_match(-1),
            C::CommandPalette => self.open_palette(),
            C::Quit => self.quit = true,
            C::SaveFile => {
                if let Some(doc) = self.editor.active_doc_mut() {
                    doc.mark_clean();
                    self.session.workspace.mark_saved(doc.file_id);
                }
            }
            C::CloseFile => {
                self.editor.close_active();
            }
            C::SwitchTabNext => self.editor.tabs.cycle(1),
            C::SwitchTabPrev => self.editor.tabs.cycle(-1),
            C::SplitEditor => {
                let direction = crate::splits::SplitDirection::Vertical;
                self.editor.splits.split(&[], direction);
            }
            C::FocusNext => self.layout.focus_next(),
            C::FocusLeft => self.layout.set_focus(PanelId::Explorer),
            C::FocusRight => self.layout.set_focus(PanelId::Chat),
            C::FocusUp => self.layout.set_focus(PanelId::Editor),
            C::FocusDown => self.layout.set_focus(PanelId::Terminal),
            C::MoveLeft => self.move_cursor(-1, 0),
            C::MoveRight => self.move_cursor(1, 0),
            C::MoveUp => self.move_cursor(0, -1),
            C::MoveDown => self.move_cursor(0, 1),
            C::PageUp => self.move_cursor(0, -10),
            C::PageDown => self.move_cursor(0, 10),
            C::GotoLine | C::JumpToLine => {
                self.mode = Mode::Command;
                self.command_line.clear();
            }
            C::Copy => {
                let copied = self.editor.active_doc().and_then(|doc| doc.copy());
                if copied.is_some() {
                    self.notifications.push(NotificationKind::Info, "Copied");
                }
            }
            C::Cut => {
                if let Some(doc) = self.editor.active_doc_mut() {
                    let _ = doc.cut();
                    self.session.workspace.mark_dirty(doc.file_id);
                }
            }
            C::Paste => {}
            C::Undo => {
                if let Some(doc) = self.editor.active_doc_mut() {
                    doc.undo();
                }
            }
            C::Redo => {
                if let Some(doc) = self.editor.active_doc_mut() {
                    doc.redo();
                }
            }
            C::ToggleExplorer => self.toggle_panel(PanelId::Explorer),
            C::ToggleTerminal => self.toggle_panel(PanelId::Terminal),
            C::ToggleUsers => self.toggle_panel(PanelId::Users),
            C::ToggleChat => self.toggle_panel(PanelId::Chat),
            C::Build => self.apply_action(Action::Build),
            C::Run => self.apply_action(Action::Run),
            C::Reconnect => {
                let username = self
                    .session
                    .state
                    .username
                    .clone()
                    .unwrap_or_else(|| "guest".into());
                self.session.connect(username);
            }
            C::OpenFile | C::RefreshExplorer => self.apply_action(Action::RefreshWorkspace),
            C::CreateFile => self.apply_action(Action::CreateFile(PathBuf::from("untitled"))),
            C::DeleteFile => {
                if let Some(entry) = self.explorer.selected_entry() {
                    let id = self
                        .session
                        .workspace
                        .state
                        .files
                        .iter()
                        .find(|(_, p)| p.as_path() == entry.path.as_path())
                        .map(|(id, _)| *id)
                        .unwrap_or_else(FileId::new);
                    self.apply_action(Action::DeleteFile(id));
                }
            }
            C::RenameFile => {
                if let Some(entry) = self.explorer.selected_entry() {
                    if let Some(id) = self
                        .session
                        .workspace
                        .state
                        .files
                        .iter()
                        .find(|(_, p)| p.as_path() == entry.path.as_path())
                        .map(|(id, _)| *id)
                    {
                        self.apply_action(Action::RenameFile(id, entry.path.clone()));
                    }
                }
            }
            C::JoinCube => {
                if let Some(id) = self.selected_cube_id() {
                    self.apply_action(Action::JoinCube(id));
                }
            }
            C::LeaveCube => self.apply_action(Action::LeaveCube),
            C::ListPlugins => self.apply_action(Action::ListPlugins),
            C::ExecuteCommand | C::ScrollUp | C::ScrollDown | C::Replace => {}
        }
    }

    fn toggle_panel(&mut self, id: PanelId) {
        match id {
            PanelId::Explorer => self.layout.toggles.explorer = !self.layout.toggles.explorer,
            PanelId::Terminal => self.layout.toggles.terminal = !self.layout.toggles.terminal,
            PanelId::Users => self.layout.toggles.users = !self.layout.toggles.users,
            PanelId::Chat => self.layout.toggles.chat = !self.layout.toggles.chat,
            PanelId::BuildOutput => self.layout.toggles.build = !self.layout.toggles.build,
            PanelId::Editor => {}
        }
        self.layout.layout(self.cols, self.rows);
    }

    fn open_palette(&mut self) {
        let mut palette = CommandPalette::new();
        palette.refresh(&self.registry);
        self.palette = Some(palette);
    }

    fn selected_cube_id(&self) -> Option<CubeId> {
        self.session
            .state
            .active_cube
            .as_ref()
            .map(|c| c.id)
            .or_else(|| self.session.state.cubes.first().map(|c| c.id))
    }

    /// Applies a user intent: locally when possible, via the session otherwise.
    pub fn apply_action(&mut self, action: Action) {
        use Action as A;
        match action {
            A::OpenFile(file_id) => {
                if self.selected_cube_id().is_none() {
                    self.notifications.push(
                        NotificationKind::Warning,
                        "Join a cube before opening files",
                    );
                    return;
                }
                let path = self
                    .session
                    .workspace
                    .path_of(file_id)
                    .cloned()
                    .unwrap_or_else(|| PathBuf::from(format!("{file_id}")));
                let content = self.session.workspace.cached(file_id).map(str::to_owned);
                let content = content.unwrap_or_default();
                let large = content.len() > crate::LARGE_FILE_THRESHOLD;
                self.editor.open(file_id, path, &content, large);
                self.layout.set_focus(PanelId::Editor);
            }
            A::SaveFile(_) => {
                if let Some(doc) = self.editor.active_doc_mut() {
                    doc.mark_clean();
                    self.session.workspace.mark_saved(doc.file_id);
                }
            }
            A::CloseFile(_) => {
                self.editor.close_active();
            }
            A::CreateFile(path) => {
                self.notifications.push(
                    NotificationKind::Info,
                    format!("Create file {path:?} (server-confirmed)"),
                );
            }
            A::DeleteFile(file_id) => {
                self.session
                    .workspace
                    .state
                    .files
                    .retain(|(id, _)| *id != file_id);
                self.notifications.push(
                    NotificationKind::Warning,
                    "Deleting files requires server confirmation",
                );
            }
            A::RenameFile(_, to) => {
                self.notifications.push(
                    NotificationKind::Info,
                    format!("Rename to {to:?} (server-confirmed)"),
                );
            }
            A::JoinCube(id) => {
                let name = self
                    .session
                    .state
                    .cubes
                    .iter()
                    .find(|c| c.id == id)
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| id.to_string());
                self.session
                    .join_cube(id, name, WorkspaceId::new(), Vec::new());
                self.layout.set_focus(PanelId::Editor);
            }
            A::LeaveCube => self.session.leave_cube(),
            A::CreateCube(name) => {
                self.session.create_cube(name);
            }
            A::Build => self.session.build("cargo build"),
            A::Run => self.session.run("cargo run"),
            A::SendChatMessage(text) => self.session.send_chat(text),
            A::SwitchTab(idx) => {
                if !self.editor.tabs.is_empty() {
                    self.editor.tabs.active = idx.min(self.editor.tabs.len().saturating_sub(1));
                }
            }
            A::CommitLocalEdit(_) => {}
            A::RefreshWorkspace => {
                self.notifications
                    .push(NotificationKind::Info, "Workspace refreshed");
            }
            A::ListPlugins => self.session.plugin_status(),
        }
    }

    /// Assembles the status-bar snapshot.
    pub fn assemble_snapshot(&mut self) {
        let s = &mut self.snapshot;
        s.connection = match self.session.state.connection {
            ConnectionState::Connected => "CONNECTED".into(),
            ConnectionState::Connecting => "CONNECTING".into(),
            ConnectionState::Disconnected => "OFFLINE".into(),
        };
        s.cube = self
            .session
            .state
            .active_cube
            .as_ref()
            .map(|c| c.name.clone());
        s.user = self.session.state.user_label();
        s.permission = None;
        s.mode = self.mode.name().to_ascii_uppercase();
        s.summary = if self.build.running {
            Some("building…".into())
        } else if !self.chat.lines.is_empty() {
            Some(format!("{} messages", self.chat.lines.len()))
        } else {
            None
        };
        let (file, language, line, column) = match self.editor.active_doc() {
            Some(doc) => {
                let (l, c) = doc.buffer.line_col(doc.cursor.offset);
                (
                    doc.path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| format!("{n}{}", if doc.dirty { " ●" } else { "" })),
                    Some(doc.language.to_string()),
                    Some(l as u64 + 1),
                    Some(c as u64 + 1),
                )
            }
            None => (None, None, None, None),
        };
        s.file = file;
        s.language = language;
        s.line = line;
        s.column = column;
    }

    /// The status-bar text for the current frame.
    pub fn status_line_text(&mut self) -> String {
        self.assemble_snapshot();
        self.snapshot.to_status_line(self.cols)
    }
}

/// Parses a `:`-command line into a [`Command`].
fn parse_command_line(line: &str, registry: &CommandRegistry) -> Option<Command> {
    let line = line.trim().trim_start_matches(':');
    if line.is_empty() {
        return None;
    }
    if let Some(direct) = Command::from_str(line) {
        return Some(direct);
    }
    registry
        .all()
        .find(|(_, spec)| {
            spec.title
                .to_ascii_lowercase()
                .contains(&line.to_ascii_lowercase())
        })
        .map(|(command, _)| *command)
}

/// Converts a theme [`StyleSpec`] to a ratatui style.
#[must_use]
pub fn to_terminal_style(spec: StyleSpec) -> ratatui::style::Style {
    use ratatui::style::{Modifier, Style};
    let mut style = Style::new();
    if let Some(fg) = spec.fg {
        style = style.fg(to_terminal_color(fg));
    }
    if let Some(bg) = spec.bg {
        style = style.bg(to_terminal_color(bg));
    }
    if spec.bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    if spec.italic {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if spec.underlined {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    if spec.dim {
        style = style.add_modifier(Modifier::DIM);
    }
    style
}

/// Converts a theme [`ColorSpec`] to a ratatui color.
#[must_use]
pub fn to_terminal_color(spec: ColorSpec) -> ratatui::style::Color {
    use ratatui::style::Color;
    match spec {
        ColorSpec::Black => Color::Black,
        ColorSpec::Red => Color::Red,
        ColorSpec::Green => Color::Green,
        ColorSpec::Yellow => Color::Yellow,
        ColorSpec::Blue => Color::Blue,
        ColorSpec::Magenta => Color::Magenta,
        ColorSpec::Cyan => Color::Cyan,
        ColorSpec::White => Color::White,
        ColorSpec::Gray => Color::Gray,
        ColorSpec::DarkGray => Color::DarkGray,
        ColorSpec::LightRed => Color::LightRed,
        ColorSpec::LightGreen => Color::LightGreen,
        ColorSpec::LightYellow => Color::LightYellow,
        ColorSpec::LightBlue => Color::LightBlue,
        ColorSpec::LightMagenta => Color::LightMagenta,
        ColorSpec::LightCyan => Color::LightCyan,
        ColorSpec::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// Renders the whole UI for one frame.
pub fn render_frame(app: &mut App, frame: &mut Frame) {
    let map = app.layout.layout(app.cols, app.rows);
    render::draw(app, frame, &map);
}

#[doc(hidden)]
pub(crate) mod render {
    use ratatui::layout::Rect as TRect;
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Block, Borders, Paragraph};

    use super::{to_terminal_style, App, ConnectionState, StyleKey};
    use crate::editor::syntax::{self, HighlightKind, SyntaxProvider};
    use crate::layout::{LayoutMap, Rect};

    fn border_rect(rect: &Rect) -> TRect {
        TRect::new(rect.x, rect.y, rect.width, rect.height)
    }

    fn panel_block<'a>(app: &'a App, title: impl Into<String>) -> Block<'a> {
        Block::bordered()
            .title(Span::styled(
                title.into(),
                to_terminal_style(app.theme.style(StyleKey::PanelTitle)),
            ))
            .style(to_terminal_style(app.theme.style(StyleKey::PanelBorder)))
    }

    /// Draws every visible panel plus overlays.
    pub(crate) fn draw(app: &mut App, frame: &mut ratatui::Frame, map: &LayoutMap) {
        render_header(app, frame, &map.header);

        if let Some(rect) = map.explorer {
            let lines: Vec<String> = app
                .explorer
                .entries
                .iter()
                .map(|e| {
                    let icon = if e.is_dir { "[d] " } else { "    " };
                    format!("{icon}{}", e.name())
                })
                .collect();
            frame.render_widget(
                Paragraph::new(lines.join("\n")).block(panel_block(app, "Explorer")),
                border_rect(&rect),
            );
        }

        render_editor(app, frame, &map.editor);

        if let Some(rect) = map.users {
            let mut names: Vec<String> = app
                .users
                .members()
                .iter()
                .map(|m| format!("• {}", m.name))
                .collect();
            if names.is_empty() {
                names.push("No members yet.".to_string());
            }
            frame.render_widget(
                Paragraph::new(names.join("\n")).block(panel_block(app, "Users")),
                border_rect(&rect),
            );
        }

        if let Some(rect) = map.chat {
            let height = (rect.height as usize).saturating_sub(4);
            let mut lines: Vec<String> = app
                .chat
                .view(height)
                .iter()
                .map(|line| match line.kind {
                    crate::chat::ChatLineKind::Message => format!("{}: {}", line.from, line.text),
                    _ => format!("── {} {}", line.from, line.text),
                })
                .collect();
            lines.push(format!("> {}", app.chat.input));
            frame.render_widget(
                Paragraph::new(lines.join("\n")).block(panel_block(app, "Chat")),
                border_rect(&rect),
            );
        }

        if let Some(rect) = map.terminal {
            let height = (rect.height as usize).saturating_sub(3);
            let lines: Vec<String> = app
                .terminal
                .view(height)
                .iter()
                .map(|l| l.text.clone())
                .collect();
            let title = if app.terminal.running {
                "Terminal ●"
            } else {
                "Terminal"
            };
            frame.render_widget(
                Paragraph::new(lines.join("\n")).block(panel_block(app, title)),
                border_rect(&rect),
            );
        }

        if let Some(rect) = map.build {
            use crate::build_panel::BuildOutcome as O;
            let status = app
                .build
                .outcome
                .map(|o| match o {
                    O::Success => "build OK".to_owned(),
                    O::Failed => "build FAILED".to_owned(),
                })
                .unwrap_or_else(|| {
                    if app.build.running {
                        "building…".to_owned()
                    } else {
                        "ready".to_owned()
                    }
                });
            let mut lines = vec![status];
            lines.extend(app.build.diagnostics.iter().take(50).map(|d| {
                let loc = d
                    .file
                    .as_ref()
                    .map(|f| format!("{}:{}", f.display(), d.line.unwrap_or(0)))
                    .unwrap_or_default();
                format!("{} {loc} {}", level_name(d.level), d.message)
            }));
            frame.render_widget(
                Paragraph::new(lines.join("\n")).block(panel_block(app, "Build")),
                border_rect(&rect),
            );
        }

        render_status(app, frame, &map.status);
        render_overlays(app, frame, map);
    }

    fn level_name(level: crate::build_panel::DiagnosticLevel) -> &'static str {
        use crate::build_panel::DiagnosticLevel::*;
        match level {
            Error => "E",
            Warning => "W",
            Info => "I",
        }
    }

    fn render_status(app: &mut App, frame: &mut ratatui::Frame, rect: &Rect) {
        let text = app.status_line_text();
        let block = Block::bordered().borders(Borders::NONE);
        frame.render_widget(Paragraph::new(text).block(block), border_rect(rect));
    }

    fn render_header(app: &App, frame: &mut ratatui::Frame, rect: &Rect) {
        use ratatui::style::Modifier;

        let brand_style = {
            let mut style = to_terminal_style(app.theme.style(StyleKey::PanelTitle));
            style = style.add_modifier(Modifier::BOLD);
            style
        };
        let body_style = to_terminal_style(app.theme.style(StyleKey::Foreground));
        let connection = match app.session.state.connection {
            ConnectionState::Connected => "CONNECTED".to_string(),
            ConnectionState::Connecting => "CONNECTING".to_string(),
            ConnectionState::Disconnected => "OFFLINE".to_string(),
        };
        let line = Line::from(vec![
            Span::styled("Nexus".to_string(), brand_style),
            Span::styled(
                format!("  ·  {}  ·  {}", app.session.state.user_label(), connection),
                body_style,
            ),
        ]);
        frame.render_widget(Paragraph::new(vec![line]), border_rect(rect));
    }

    fn render_editor(app: &mut App, frame: &mut ratatui::Frame, rect: &Rect) {
        let title = app
            .session
            .state
            .active_cube
            .as_ref()
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "Editor".into());
        let block = panel_block(app, title);
        let area = border_rect(rect);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let text_area = TRect::new(inner.x, inner.y, inner.width, inner.height);
        if app.editor.active_doc().is_none() {
            render_welcome(app, frame, text_area);
        } else {
            render_editor_lines(app, frame, text_area);
        }
    }

    /// Renders the first-launch welcome content when no document is open.
    fn render_welcome(app: &App, frame: &mut ratatui::Frame, area: TRect) {
        let lines = welcome_lines(app);
        frame.render_widget(Paragraph::new(lines), area);
    }

    /// Builds the welcome content shown before any file is open: branding,
    /// connection status, Cube status, next steps, and shortcuts. Derived
    /// purely from current state; no fake actions are introduced.
    fn welcome_lines(app: &App) -> Vec<Line<'static>> {
        use ratatui::style::Modifier;

        let mut brand_style = to_terminal_style(app.theme.style(StyleKey::PanelTitle));
        brand_style = brand_style.add_modifier(Modifier::BOLD);
        let body_style = to_terminal_style(app.theme.style(StyleKey::Foreground));
        let heading_style = brand_style;

        let mut lines: Vec<Line<'static>> = Vec::new();
        lines.push(Line::from(Span::styled("Nexus".to_string(), brand_style)));
        lines.push(Line::from(Span::styled(
            "Nexus workspace".to_string(),
            body_style,
        )));

        let (status_text, status_style) = match app.session.state.connection {
            ConnectionState::Connected => (
                format!(
                    "Connected — signed in as {}",
                    app.session.state.user_label()
                ),
                to_terminal_style(app.theme.style(StyleKey::NotificationSuccess)),
            ),
            ConnectionState::Connecting => (
                "Connecting — waiting for server…".to_string(),
                to_terminal_style(app.theme.style(StyleKey::NotificationWarning)),
            ),
            ConnectionState::Disconnected => (
                "Offline — not connected".to_string(),
                to_terminal_style(app.theme.style(StyleKey::NotificationError)),
            ),
        };
        lines.push(Line::from(Span::styled(
            format!("Status: {status_text}"),
            status_style,
        )));

        let cube_name = app
            .session
            .state
            .active_cube
            .as_ref()
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "none".to_string());
        lines.push(Line::from(Span::styled(
            format!("Cube: {cube_name}"),
            body_style,
        )));
        lines.push(Line::from(Span::styled(" ".to_string(), body_style)));

        if app.session.state.active_cube.is_none() {
            lines.push(Line::from(Span::styled(
                "You are not inside a Cube yet.".to_string(),
                to_terminal_style(app.theme.style(StyleKey::Warning)),
            )));
            lines.push(Line::from(Span::styled(
                "Next step: open the command palette with Ctrl-P and run the join-cube command."
                    .to_string(),
                body_style,
            )));
        } else {
            lines.push(Line::from(Span::styled(
                "No file is open in this Cube.".to_string(),
                body_style,
            )));
        }

        lines.push(Line::from(Span::styled(" ".to_string(), body_style)));
        lines.push(Line::from(Span::styled(
            "Shortcuts".to_string(),
            heading_style,
        )));
        lines.push(Line::from(Span::styled(
            "  Ctrl-P  Command palette    Ctrl-S  Save file",
            body_style,
        )));
        lines.push(Line::from(Span::styled(
            "  Ctrl-B  Build project        Ctrl-O  Open file",
            body_style,
        )));
        lines.push(Line::from(Span::styled(
            "  Ctrl-W  Close tab            Ctrl-Q  Quit",
            body_style,
        )));
        lines.push(Line::from(Span::styled(
            "  Ctrl-R  Reconnect              /  Search",
            body_style,
        )));
        lines.push(Line::from(Span::styled(
            "  e  Toggle Explorer   t  Toggle Terminal   u  Toggle Users   c  Toggle Chat",
            body_style,
        )));
        lines.push(Line::from(Span::styled(
            "  h j k l / arrows  Move cursor     Tab  Focus next panel",
            body_style,
        )));

        lines
    }

    fn render_editor_lines(app: &mut App, frame: &mut ratatui::Frame, area: TRect) {
        let Some(doc) = app.editor.active_doc() else {
            return;
        };
        let (cursor_line, cursor_col) = doc.buffer.line_col(doc.cursor.offset);
        let height = area.height as usize;
        let first = cursor_line.saturating_sub(height / 2);
        let provider = syntax::provider_for(doc.language);
        let fg = app.theme.style(StyleKey::Foreground);
        let cursor_bg = app.theme.style(StyleKey::Cursor);
        let gutter_style = app.theme.style(StyleKey::LineNumber);
        let tab_label = app
            .editor
            .tabs
            .active_tab()
            .map(|t| t.label.clone())
            .unwrap_or_default();

        let mut lines: Vec<Line<'static>> = Vec::new();
        lines.push(Line::from(vec![Span::styled(
            format!("📄 {tab_label}"),
            to_terminal_style(app.theme.style(StyleKey::TabActive)),
        )]));
        for line_no in first..first + height.saturating_sub(1).max(1) {
            let text = doc.buffer.line_text(line_no);
            let chars: Vec<char> = text.chars().collect();
            let kinds = provider
                .as_ref()
                .map(|p| p.highlight_line(&text))
                .unwrap_or_default();

            let mut spans: Vec<Span<'static>> = Vec::new();
            spans.push(Span::styled(
                format!("{:>4} ", line_no + 1),
                to_terminal_style(gutter_style),
            ));
            let mut run_start = 0usize;
            let mut run_style = to_terminal_style(fg);
            for i in 0..chars.len() {
                let kind = kinds
                    .iter()
                    .find(|s| i >= s.start && i < s.end)
                    .map(|s| s.kind);
                let is_cursor = line_no == cursor_line && i == cursor_col;
                let style = if is_cursor {
                    to_terminal_style(cursor_bg)
                } else {
                    to_terminal_style(style_for_kind(app, kind))
                };
                if style != run_style {
                    if i > run_start {
                        let run: String = chars[run_start..i].iter().collect();
                        spans.push(Span::styled(run, run_style));
                    }
                    run_start = i;
                    run_style = style;
                }
            }
            if run_start < chars.len() {
                let run: String = chars[run_start..].iter().collect();
                spans.push(Span::styled(run, run_style));
            }
            lines.push(Line::from(spans));
        }
        frame.render_widget(Paragraph::new(lines), area);
    }

    fn style_for_kind(app: &App, kind: Option<HighlightKind>) -> crate::theme::StyleSpec {
        let key = match kind {
            Some(HighlightKind::Keyword) => StyleKey::Keyword,
            Some(HighlightKind::String) => StyleKey::String,
            Some(HighlightKind::Number) => StyleKey::Number,
            Some(HighlightKind::Comment) => StyleKey::Comment,
            Some(HighlightKind::Type) | Some(HighlightKind::Function) => StyleKey::Foreground,
            Some(HighlightKind::Operator) | Some(HighlightKind::Punctuation) => {
                StyleKey::Foreground
            }
            None => StyleKey::Foreground,
        };
        app.theme.style(key)
    }

    fn render_overlays(app: &mut App, frame: &mut ratatui::Frame, _map: &LayoutMap) {
        if let Some(palette) = &mut app.palette {
            let center = frame.area();
            let width = center.width.min(70);
            let height = 12.min(center.height.saturating_sub(2));
            let rect = TRect::new(
                center.x + center.width.saturating_sub(width) / 2,
                center.y + center.height.saturating_sub(height) / 3,
                width,
                height,
            );
            let mut lines = vec![format!("> {}", palette.query)];
            lines.extend(
                palette
                    .results
                    .iter()
                    .take(9)
                    .map(|r| format!("  {}", r.title)),
            );
            let block = Block::bordered()
                .title("Commands")
                .style(to_terminal_style(app.theme.style(StyleKey::PanelBorder)));
            frame.render_widget(Paragraph::new(lines.join("\n")).block(block), rect);
        }

        if let Some(dialog) = app.dialogs.active() {
            let center = frame.area();
            let width = center.width.min(60);
            let height = 8.min(center.height.saturating_sub(2));
            let rect = TRect::new(
                center.x + center.width.saturating_sub(width) / 2,
                center.y + center.height.saturating_sub(height) / 3,
                width,
                height,
            );
            let mut lines = vec![dialog.title.clone(), dialog.message.clone()];
            if dialog.kind == crate::dialogs::DialogKind::Input {
                lines.push(app.dialog_text.clone());
            }
            lines.push("[Enter] confirm  [Esc] cancel".into());
            let block = Block::bordered()
                .title("Nexus")
                .style(to_terminal_style(app.theme.style(StyleKey::PanelBorder)));
            frame.render_widget(Paragraph::new(lines.join("\n")).block(block), rect);
        }

        if !app.notifications.is_empty() {
            let center = frame.area();
            let lines: Vec<Line<'static>> = app
                .notifications
                .newest_first()
                .take(3)
                .map(|n| {
                    let key = match n.kind {
                        crate::notifications::NotificationKind::Error => {
                            StyleKey::NotificationError
                        }
                        crate::notifications::NotificationKind::Warning => {
                            StyleKey::NotificationWarning
                        }
                        _ => StyleKey::NotificationInfo,
                    };
                    Line::from(Span::styled(
                        n.text.clone(),
                        to_terminal_style(app.theme.style(key)),
                    ))
                })
                .collect();
            let w = center.width.min(60);
            let rect = TRect::new(
                center.x + center.width.saturating_sub(w),
                center.y,
                w,
                lines.len() as u16,
            );
            frame.render_widget(Paragraph::new(lines), rect);
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use nexus_client::app::state::CubeInfo;
        use nexus_client::SessionService;
        use nexus_core::ids::CubeId;

        fn welcome_text(app: &App) -> String {
            welcome_lines(app)
                .iter()
                .map(|l| {
                    l.spans
                        .iter()
                        .map(|s| (*s.content).to_string())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect::<Vec<_>>()
                .join("\n")
        }

        #[test]
        fn welcome_lists_branding_status_and_shortcuts() {
            let app = App::new(SessionService::new("ws://localhost:9000"));
            let text = welcome_text(&app);
            assert!(text.contains("Nexus"));
            assert!(text.contains("Status:"));
            assert!(text.contains("Shortcuts"));
            assert!(text.contains("Ctrl-P"));
            assert!(text.contains("Ctrl-Q"));
        }

        #[test]
        fn welcome_offers_next_step_when_no_cube_is_joined() {
            let app = App::new(SessionService::new("ws://localhost:9000"));
            let text = welcome_text(&app);
            assert!(text.contains("not inside a Cube yet"));
        }

        #[test]
        fn welcome_reflects_joined_cube_state() {
            let mut app = App::new(SessionService::new("ws://localhost:9000"));
            app.session.state.active_cube = Some(CubeInfo {
                id: CubeId::new(),
                name: "backend-api".to_string(),
            });
            let text = welcome_text(&app);
            assert!(text.contains("backend-api"));
            assert!(!text.contains("not inside a Cube yet"));
        }
    }
}
