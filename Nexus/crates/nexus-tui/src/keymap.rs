//! Configurable modal keymap.
//!
//! Bindings are stored centrally; widgets never interpret raw keys themselves.

use crate::commands::Command;
use crate::input::Key;

/// Editing mode. Determines which bindings are active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    Normal,
    Insert,
    Search,
    Command,
}

impl Mode {
    /// Stable machine name, used for persistence and tests.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Insert => "insert",
            Self::Search => "search",
            Self::Command => "command",
        }
    }
}

/// A single key-to-command mapping active in one [`Mode`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub mode: Mode,
    pub key: Key,
    pub command: Command,
}

impl Binding {
    /// Constructs a binding for `mode`, `key`, `command`.
    #[must_use]
    pub const fn new(mode: Mode, key: Key, command: Command) -> Self {
        Self { mode, key, command }
    }
}

/// Central keymap storage. Later bindings win for the same (mode, key).
#[derive(Debug)]
pub struct Keymap {
    bindings: Vec<Binding>,
}

impl Keymap {
    /// Binds `key` to `command` within `mode`.
    pub fn set(&mut self, mode: Mode, key: Key, command: Command) {
        self.bindings.retain(|b| !(b.mode == mode && b.key == key));
        self.bindings.push(Binding { mode, key, command });
    }

    /// Removes the binding for (mode, key), if any.
    pub fn unset(&mut self, mode: Mode, key: Key) {
        self.bindings.retain(|b| !(b.mode == mode && b.key == key));
    }

    /// Finds the command bound to `key` in `mode`.
    #[must_use]
    pub fn resolve(&self, mode: Mode, key: Key) -> Option<Command> {
        self.bindings
            .iter()
            .rev()
            .find(|b| b.mode == mode && b.key == key)
            .map(|b| b.command)
    }

    /// All current bindings (used for persistence and help).
    pub fn bindings(&self) -> impl Iterator<Item = &Binding> {
        self.bindings.iter()
    }

    /// Parses a `"<mode>:<key>=<command>"` spec into a binding.
    #[must_use]
    pub fn parse_spec(spec: &str) -> Option<(Binding, String)> {
        let (mode_part, rest) = spec.split_once(':')?;
        let (key_part, command_part) = rest.split_once('=')?;
        let mode = parse_mode(mode_part)?;
        let key = Key::parse(key_part)?;
        let command = Command::from_str(command_part)?;
        Some((Binding { mode, key, command }, command_part.to_string()))
    }

    /// Applies user-provided specs over the defaults.
    pub fn apply_specs(&mut self, specs: &[String]) -> Result<(), String> {
        for spec in specs {
            match Self::parse_spec(spec) {
                Some((binding, _)) => self.set(binding.mode, binding.key, binding.command),
                None => return Err(format!("invalid keybinding spec: {spec}")),
            }
        }
        Ok(())
    }
}

impl Default for Keymap {
    /// The built-in default bindings.
    fn default() -> Self {
        use crate::commands::Command::*;
        use crate::input::Key::*;
        let mut km = Keymap {
            bindings: Vec::new(),
        };
        for (key, command) in [
            (Ctrl('p'), CommandPalette),
            (Ctrl('s'), SaveFile),
            (Ctrl('b'), Build),
            (Ctrl('o'), OpenFile),
            (Ctrl('q'), Quit),
            (Tab, FocusNext),
            (Ctrl('h'), FocusLeft),
            (Ctrl('j'), FocusDown),
            (Ctrl('k'), FocusUp),
            (Ctrl('l'), FocusRight),
            (Char('h'), MoveLeft),
            (Char('j'), MoveDown),
            (Char('k'), MoveUp),
            (Char('l'), MoveRight),
            (Up, MoveUp),
            (Down, MoveDown),
            (Left, MoveLeft),
            (Right, MoveRight),
            (Key::PageUp, Command::PageUp),
            (Key::PageDown, Command::PageDown),
            (Char('e'), ToggleExplorer),
            (Char('t'), ToggleTerminal),
            (Char('u'), ToggleUsers),
            (Char('c'), ToggleChat),
            (Char('/'), Search),
            (Char('r'), Reconnect),
            (Key::Enter, Command::Enter),
            (Ctrl('n'), JumpToLine),
            (Ctrl('w'), CloseFile),
            (Alt('w'), SwitchTabNext),
            (Ctrl(' '), SwitchTabNext),
            (Char('s'), SplitEditor), // placeholders; refined per-screen later
        ] {
            km.set(Mode::Normal, key, command);
        }
        km.set(Mode::Insert, Esc, Command::ModeNormal);
        km.set(Mode::Search, Esc, Command::ModeNormal);
        km.set(Mode::Search, Key::Enter, Command::SearchNext);
        km.set(Mode::Search, Char('n'), Command::SearchNext);
        km.set(Mode::Search, Char('N'), Command::SearchPrev);
        km.set(Mode::Command, Esc, Command::ModeNormal);
        km.set(Mode::Command, Key::Enter, Command::ExecuteCommand);
        km
    }
}

fn parse_mode(s: &str) -> Option<Mode> {
    match s.trim().to_ascii_lowercase().as_str() {
        "normal" | "n" => Some(Mode::Normal),
        "insert" | "i" => Some(Mode::Insert),
        "search" | "s" => Some(Mode::Search),
        "command" | "c" => Some(Mode::Command),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_bindings_resolve() {
        let km = Keymap::default();
        assert_eq!(
            km.resolve(Mode::Normal, Key::Ctrl('p')),
            Some(Command::CommandPalette)
        );
        assert_eq!(
            km.resolve(Mode::Normal, Key::Char('j')),
            Some(Command::MoveDown)
        );
        assert_eq!(km.resolve(Mode::Normal, Key::Char('x')), None);
        assert_eq!(
            km.resolve(Mode::Insert, Key::Esc),
            Some(Command::ModeNormal)
        );
    }

    #[test]
    fn later_bindings_win() {
        let mut km = Keymap::default();
        km.set(Mode::Normal, Key::Char('j'), Command::ToggleChat);
        assert_eq!(
            km.resolve(Mode::Normal, Key::Char('j')),
            Some(Command::ToggleChat)
        );
        km.unset(Mode::Normal, Key::Char('j'));
        assert_eq!(km.resolve(Mode::Normal, Key::Char('j')), None);
    }

    #[test]
    fn spec_parsing_and_application() {
        let spec = "normal:ctrl-s=save".to_string();
        let mut km = Keymap::default();
        km.apply_specs(&[spec]).unwrap();
        assert_eq!(
            km.resolve(Mode::Normal, Key::Ctrl('s')),
            Some(Command::SaveFile)
        );
        assert!(km.apply_specs(&["bogus".into()]).is_err());
    }

    #[test]
    fn mode_names_are_stable() {
        assert_eq!(Mode::Normal.name(), "normal");
        assert_eq!(Mode::Command.name(), "command");
    }
}
