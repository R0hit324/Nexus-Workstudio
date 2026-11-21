//! Centralized command registry.
//!
//! All user-executable operations are [`Command`]s registered here. Widgets emit
//! commands through the keymap/palette; the application dispatches them. Future
//! plugins register additional commands without touching the dispatcher.

use std::collections::HashMap;

/// Every built-in command. The registry provides titles and descriptions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Command {
    OpenFile,
    CloseFile,
    SaveFile,
    Search,
    Replace,
    SwitchTabNext,
    SwitchTabPrev,
    SplitEditor,
    ToggleExplorer,
    ToggleTerminal,
    ToggleUsers,
    ToggleChat,
    Build,
    Run,
    Reconnect,
    Quit,
    CommandPalette,
    FocusNext,
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    PageUp,
    PageDown,
    GotoLine,
    JumpToLine,
    ScrollUp,
    ScrollDown,
    Copy,
    Cut,
    Paste,
    Undo,
    Redo,
    SearchNext,
    SearchPrev,
    Enter,
    ModeNormal,
    ModeInsert,
    ExecuteCommand,
    CreateFile,
    DeleteFile,
    RenameFile,
    RefreshExplorer,
    JoinCube,
    LeaveCube,
    ListPlugins,
}

impl Command {
    /// Resolves a stable lowercase name to a command (palette/persistence).
    #[must_use]
    pub fn from_str(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        Some(self::ALL.iter().find(|c| c.name() == name)?.to_owned())
    }
}

use crate::commands::Command as C;

const ALL: &[C] = &[
    C::OpenFile,
    C::CloseFile,
    C::SaveFile,
    C::Search,
    C::Replace,
    C::SwitchTabNext,
    C::SwitchTabPrev,
    C::SplitEditor,
    C::ToggleExplorer,
    C::ToggleTerminal,
    C::ToggleUsers,
    C::ToggleChat,
    C::Build,
    C::Run,
    C::Reconnect,
    C::Quit,
    C::CommandPalette,
    C::FocusNext,
    C::FocusLeft,
    C::FocusRight,
    C::FocusUp,
    C::FocusDown,
    C::MoveUp,
    C::MoveDown,
    C::MoveLeft,
    C::MoveRight,
    C::PageUp,
    C::PageDown,
    C::GotoLine,
    C::JumpToLine,
    C::ScrollUp,
    C::ScrollDown,
    C::Copy,
    C::Cut,
    C::Paste,
    C::Undo,
    C::Redo,
    C::SearchNext,
    C::SearchPrev,
    C::Enter,
    C::ModeNormal,
    C::ModeInsert,
    C::ExecuteCommand,
    C::CreateFile,
    C::DeleteFile,
    C::RenameFile,
    C::RefreshExplorer,
    C::JoinCube,
    C::LeaveCube,
    C::ListPlugins,
];

impl Command {
    /// Stable machine name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::OpenFile => "open-file",
            Self::CloseFile => "close-file",
            Self::SaveFile => "save",
            Self::Search => "search",
            Self::Replace => "replace",
            Self::SwitchTabNext => "tab-next",
            Self::SwitchTabPrev => "tab-prev",
            Self::SplitEditor => "split-editor",
            Self::ToggleExplorer => "toggle-explorer",
            Self::ToggleTerminal => "toggle-terminal",
            Self::ToggleUsers => "toggle-users",
            Self::ToggleChat => "toggle-chat",
            Self::Build => "build",
            Self::Run => "run",
            Self::Reconnect => "reconnect",
            Self::Quit => "quit",
            Self::CommandPalette => "command-palette",
            Self::FocusNext => "focus-next",
            Self::FocusLeft => "focus-left",
            Self::FocusRight => "focus-right",
            Self::FocusUp => "focus-up",
            Self::FocusDown => "focus-down",
            Self::MoveUp => "move-up",
            Self::MoveDown => "move-down",
            Self::MoveLeft => "move-left",
            Self::MoveRight => "move-right",
            Self::PageUp => "page-up",
            Self::PageDown => "page-down",
            Self::GotoLine => "goto-line",
            Self::JumpToLine => "jump-to-line",
            Self::ScrollUp => "scroll-up",
            Self::ScrollDown => "scroll-down",
            Self::Copy => "copy",
            Self::Cut => "cut",
            Self::Paste => "paste",
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::SearchNext => "search-next",
            Self::SearchPrev => "search-prev",
            Self::Enter => "enter",
            Self::ModeNormal => "mode-normal",
            Self::ModeInsert => "mode-insert",
            Self::ExecuteCommand => "execute-command",
            Self::CreateFile => "create-file",
            Self::DeleteFile => "delete-file",
            Self::RenameFile => "rename-file",
            Self::RefreshExplorer => "refresh-explorer",
            Self::JoinCube => "join-cube",
            Self::LeaveCube => "leave-cube",
            Self::ListPlugins => "plugin-status",
        }
    }
}

/// Human-facing metadata for a registered command.
#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub title: &'static str,
    pub description: &'static str,
}

impl CommandSpec {
    const fn new(title: &'static str, description: &'static str) -> Self {
        Self { title, description }
    }
}

/// The registry all command sources (keymap, palette, plugins) consult.
#[derive(Debug, Default)]
pub struct CommandRegistry {
    specs: HashMap<Command, CommandSpec>,
}

impl CommandRegistry {
    /// The built-in registry covering every [`Command`] variant.
    pub fn default() -> Self {
        use Command::*;
        let mut specs = HashMap::new();
        for command in ALL {
            let spec = match command {
                OpenFile => CommandSpec::new("Open File", "Open a file in a new tab"),
                CloseFile => CommandSpec::new("Close File", "Close the active tab"),
                SaveFile => CommandSpec::new("Save", "Save the active file"),
                Search => CommandSpec::new("Search", "Search the current file"),
                Replace => CommandSpec::new("Replace", "Replace matches in the current file"),
                SwitchTabNext => CommandSpec::new("Next Tab", "Switch to the next tab"),
                SwitchTabPrev => CommandSpec::new("Previous Tab", "Switch to the previous tab"),
                SplitEditor => CommandSpec::new("Split Editor", "Split the editor pane"),
                ToggleExplorer => {
                    CommandSpec::new("Toggle Explorer", "Show or hide the file explorer")
                }
                ToggleTerminal => {
                    CommandSpec::new("Toggle Terminal", "Show or hide the terminal panel")
                }
                ToggleUsers => CommandSpec::new("Toggle Users", "Show or hide the users panel"),
                ToggleChat => CommandSpec::new("Toggle Chat", "Show or hide the chat panel"),
                Build => CommandSpec::new("Build Project", "Run the project build"),
                Run => CommandSpec::new("Run Program", "Run the program"),
                Reconnect => CommandSpec::new("Reconnect", "Reconnect to the server"),
                Quit => CommandSpec::new("Quit", "Quit Nexus"),
                CommandPalette => CommandSpec::new("Command Palette", "Search and run commands"),
                FocusNext => CommandSpec::new("Focus Next", "Move focus to the next panel"),
                FocusLeft => CommandSpec::new("Focus Left", "Focus the panel to the left"),
                FocusRight => CommandSpec::new("Focus Right", "Focus the panel to the right"),
                FocusUp => CommandSpec::new("Focus Up", "Focus the panel above"),
                FocusDown => CommandSpec::new("Focus Down", "Focus the panel below"),
                MoveUp => CommandSpec::new("Cursor Up", "Move the cursor up"),
                MoveDown => CommandSpec::new("Cursor Down", "Move the cursor down"),
                MoveLeft => CommandSpec::new("Cursor Left", "Move the cursor left"),
                MoveRight => CommandSpec::new("Cursor Right", "Move the cursor right"),
                PageUp => CommandSpec::new("Page Up", "Scroll one page up"),
                PageDown => CommandSpec::new("Page Down", "Scroll one page down"),
                GotoLine => CommandSpec::new("Go to Line", "Jump to a line number"),
                JumpToLine => CommandSpec::new("Jump to Line", "Prompt for a line number"),
                ScrollUp => CommandSpec::new("Scroll Up", "Scroll the focused panel up"),
                ScrollDown => CommandSpec::new("Scroll Down", "Scroll the focused panel down"),
                Copy => CommandSpec::new("Copy", "Copy the selection"),
                Cut => CommandSpec::new("Cut", "Cut the selection"),
                Paste => CommandSpec::new("Paste", "Paste from the clipboard"),
                Undo => CommandSpec::new("Undo", "Undo the last edit"),
                Redo => CommandSpec::new("Redo", "Redo the last undone edit"),
                SearchNext => CommandSpec::new("Next Match", "Jump to the next search match"),
                SearchPrev => {
                    CommandSpec::new("Previous Match", "Jump to the previous search match")
                }
                Enter => CommandSpec::new("Confirm", "Confirm the current action"),
                ModeNormal => CommandSpec::new("Normal Mode", "Return to Normal mode"),
                ModeInsert => CommandSpec::new("Insert Mode", "Enter Insert mode"),
                ExecuteCommand => CommandSpec::new("Execute", "Execute the current command line"),
                CreateFile => CommandSpec::new("Create File", "Create a new file"),
                DeleteFile => CommandSpec::new("Delete File", "Delete the selected file"),
                RenameFile => CommandSpec::new("Rename File", "Rename the selected file"),
                RefreshExplorer => CommandSpec::new("Refresh", "Refresh the file explorer"),
                JoinCube => CommandSpec::new("Join Cube", "Join the selected Cube"),
                LeaveCube => CommandSpec::new("Leave Cube", "Leave the current Cube"),
                ListPlugins => CommandSpec::new("Plugin Status", "List server plugins and states"),
            };
            specs.insert(*command, spec);
        }
        Self { specs }
    }

    /// Registers a command, overriding any existing spec. This is the plugin
    /// extension point: plugins call `register` with their own command id.
    pub fn register(&mut self, command: Command, spec: CommandSpec) {
        self.specs.insert(command, spec);
    }

    /// Metedata for `command`, or `None` for registrable-but-unknown ids.
    #[must_use]
    pub fn get(&self, command: Command) -> Option<&CommandSpec> {
        self.specs.get(&command)
    }

    /// Every registered command (ordered by insertion) for the palette.
    pub fn all(&self) -> impl Iterator<Item = (&Command, &CommandSpec)> {
        self.specs.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_covers_every_command_with_a_title() {
        let reg = CommandRegistry::default();
        for command in ALL {
            assert!(reg.get(*command).is_some(), "missing spec for {command:?}");
        }
    }

    #[test]
    fn command_names_parse_and_resolve() {
        assert_eq!(Command::from_str("save"), Some(Command::SaveFile));
        assert_eq!(
            Command::from_str("COMMAND-PALETTE"),
            Some(Command::CommandPalette)
        );
        assert_eq!(Command::from_str("nope"), None);
        for command in ALL {
            assert_eq!(Command::from_str(command.name()), Some(*command));
        }
    }

    #[test]
    fn registration_can_extend_the_registry() {
        let mut reg = CommandRegistry::default();
        let n = reg.all().count();
        reg.register(
            Command::SaveFile,
            CommandSpec::new("Persist", "Save with extra flavor"),
        );
        assert_eq!(reg.all().count(), n); // override, not duplicate
    }
}
