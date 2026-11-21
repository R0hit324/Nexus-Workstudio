//! Unified input event model.
//!
//! Raw terminal events are translated by [`InputTranslator`] into application
//! level [`UiEvent`]s so that widgets and keymaps never depend on crossterm.

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton as CTMouseButton,
    MouseEvent as CTMouse, MouseEventKind,
};
use std::fmt;

/// A canonical, terminal-independent key. The [`crate::Keymap`] matches on this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Char(char),
    Ctrl(char),
    Alt(char),
    Enter,
    Esc,
    Tab,
    BackTab,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
}

impl Key {
    /// Parses a human-readable binding such as `Ctrl-P`, `F5`, `Enter`, `h`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let lower = s.to_ascii_lowercase();
        match lower.as_str() {
            "enter" => return Some(Self::Enter),
            "esc" | "escape" => return Some(Self::Esc),
            "tab" => return Some(Self::Tab),
            "backtab" | "shift-tab" => return Some(Self::BackTab),
            "backspace" | "bs" => return Some(Self::Backspace),
            "delete" | "del" => return Some(Self::Delete),
            "up" => return Some(Self::Up),
            "down" => return Some(Self::Down),
            "left" => return Some(Self::Left),
            "right" => return Some(Self::Right),
            "home" => return Some(Self::Home),
            "end" => return Some(Self::End),
            "pageup" | "pgup" => return Some(Self::PageUp),
            "pagedown" | "pgdn" => return Some(Self::PageDown),
            _ => {}
        }
        if let Some(f) = lower.strip_prefix('f') {
            if let Ok(n) = f.parse::<u8>() {
                if (1..=24).contains(&n) {
                    return Some(Self::F(n));
                }
            }
        }
        // Modified keys: "Ctrl-P", "Alt-x".
        if let Some((modifier, rest)) = lower.split_once('-') {
            let mut chars = rest.chars();
            let ch = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            return Some(match modifier {
                "ctrl" => Self::Ctrl(ch),
                "alt" => Self::Alt(ch),
                _ => return None,
            });
        }
        // Bare printable character.
        let mut chars = s.chars();
        let ch = chars.next()?;
        if chars.next().is_some() {
            return None;
        }
        Some(Self::Char(ch))
    }

    /// The canonical single-character representation of `Ever`?, lowercase.
    /// Used by tests and serialization.
    #[must_use]
    pub fn label(self) -> String {
        match self {
            Self::Char(c) => c.to_string(),
            Self::Ctrl(c) => format!("Ctrl-{}", to_upper(c)),
            Self::Alt(c) => format!("Alt-{}", to_upper(c)),
            Self::F(n) => format!("F{n}"),
            _ => format!("{self:?}"),
        }
    }
}

fn to_upper(c: char) -> char {
    c.to_ascii_uppercase()
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label())
    }
}

/// Kind of a mouse interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseKind {
    Click,
    DoubleClick,
    Drag,
    ScrollUp,
    ScrollDown,
    Release,
}

/// Mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    None,
}

/// A terminal mouse event, independent of crossterm types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseEvent {
    pub kind: MouseKind,
    pub button: MouseButton,
    pub row: u16,
    pub col: u16,
}

/// Application-level events produced by the input layer.
#[derive(Debug, PartialEq, Eq)]
pub enum UiEvent {
    Key(Key),
    Mouse(MouseEvent),
    Resize(u16, u16),
    Tick,
    Network(nexus_client::network::NetworkEvent),
    Command(crate::commands::Command),
    Action(crate::actions::Action),
}

/// Translates raw `crossterm` terminal events into [`UiEvent`]s.
pub struct InputTranslator;

impl InputTranslator {
    /// Converts a terminal event, dropping key *releases* and unknown events.
    #[must_use]
    pub fn translate(event: Event) -> Option<UiEvent> {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                Self::from_key(key).map(UiEvent::Key)
            }
            Event::Key(_) => None,
            Event::Mouse(mouse) => Self::from_mouse(mouse).map(UiEvent::Mouse),
            Event::Resize(cols, rows) => Some(UiEvent::Resize(cols, rows)),
            Event::Paste(text) => {
                // Pasting a single line is treated as a key; bigger text is
                // routed by the application as an action.
                if text.chars().count() == 1 {
                    text.chars().next().map(|c| UiEvent::Key(Key::Char(c)))
                } else {
                    None
                }
            }
            _ => None, // FocusGained/FocusLost/None
        }
    }

    fn from_key(key: KeyEvent) -> Option<Key> {
        let code = key.code;
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return char_code(code).map(Key::Ctrl);
        }
        if key.modifiers.contains(KeyModifiers::ALT) {
            return char_code(code).map(Key::Alt);
        }
        match code {
            KeyCode::Char(c) => Some(Key::Char(c)),
            KeyCode::Enter => Some(Key::Enter),
            KeyCode::Esc => Some(Key::Esc),
            KeyCode::Tab => Some(Key::Tab),
            KeyCode::BackTab => Some(Key::BackTab),
            KeyCode::Backspace => Some(Key::Backspace),
            KeyCode::Delete => Some(Key::Delete),
            KeyCode::Up => Some(Key::Up),
            KeyCode::Down => Some(Key::Down),
            KeyCode::Left => Some(Key::Left),
            KeyCode::Right => Some(Key::Right),
            KeyCode::Home => Some(Key::Home),
            KeyCode::End => Some(Key::End),
            KeyCode::PageUp => Some(Key::PageUp),
            KeyCode::PageDown => Some(Key::PageDown),
            KeyCode::F(n) => Some(Key::F(n)),
            _ => None,
        }
    }

    fn from_mouse(mouse: CTMouse) -> Option<MouseEvent> {
        let kind = match mouse.kind {
            MouseEventKind::Down(_) => MouseKind::Click,
            MouseEventKind::Drag(_) => MouseKind::Drag,
            MouseEventKind::ScrollUp => MouseKind::ScrollUp,
            MouseEventKind::ScrollDown => MouseKind::ScrollDown,
            MouseEventKind::Up(_) => MouseKind::Release,
            _ => return None, // moved events
        };
        let button = match mouse.kind {
            MouseEventKind::Down(CTMouseButton::Left)
            | MouseEventKind::Drag(CTMouseButton::Left)
            | MouseEventKind::Up(CTMouseButton::Left) => MouseButton::Left,
            MouseEventKind::Down(CTMouseButton::Right)
            | MouseEventKind::Drag(CTMouseButton::Right)
            | MouseEventKind::Up(CTMouseButton::Right) => MouseButton::Right,
            MouseEventKind::Down(CTMouseButton::Middle)
            | MouseEventKind::Drag(CTMouseButton::Middle)
            | MouseEventKind::Up(CTMouseButton::Middle) => MouseButton::Middle,
            _ => MouseButton::None,
        };
        Some(MouseEvent {
            kind,
            button,
            row: mouse.row,
            col: mouse.column,
        })
    }
}

fn char_code(code: KeyCode) -> Option<char> {
    match code {
        KeyCode::Char(c) => Some(c),
        KeyCode::Enter => Some('\r'),
        KeyCode::Tab => Some('\t'),
        KeyCode::Backspace => Some('\u{7f}'),
        KeyCode::Esc => Some('\x1b'),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_bindings_parse() {
        assert_eq!(Key::parse("Ctrl-P"), Some(Key::Ctrl('p')));
        assert_eq!(Key::parse("F5"), Some(Key::F(5)));
        assert_eq!(Key::parse("Enter"), Some(Key::Enter));
        assert_eq!(Key::parse("h"), Some(Key::Char('h')));
        assert_eq!(Key::parse("nope"), None);
        assert_eq!(Key::parse(""), None);
    }

    #[test]
    fn key_labels_roundtrip() {
        let cases = [
            (Key::Ctrl('p'), "Ctrl-P"),
            (Key::Alt('x'), "Alt-X"),
            (Key::F(12), "F12"),
            (Key::Enter, "Enter"),
        ];
        for (key, label) in cases {
            assert_eq!(key.label(), label);
        }
    }

    #[test]
    fn key_events_translate() {
        use crossterm::event::{KeyCode as KC, KeyModifiers as KM};
        let press = |code| Event::Key(KeyEvent::new(code, KM::NONE));
        assert_eq!(
            InputTranslator::translate(press(KC::Char('a'))),
            Some(UiEvent::Key(Key::Char('a')))
        );
        assert_eq!(
            InputTranslator::translate(Event::Key(KeyEvent::new(KC::Char('p'), KM::CONTROL))),
            Some(UiEvent::Key(Key::Ctrl('p')))
        );
        assert_eq!(
            InputTranslator::translate(press(KC::Enter)),
            Some(UiEvent::Key(Key::Enter))
        );
        assert_eq!(
            InputTranslator::translate(Event::Resize(120, 40)),
            Some(UiEvent::Resize(120, 40))
        );
    }

    #[test]
    fn key_releases_are_ignored() {
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        // A release event must never reach the UI.
        assert_eq!(InputTranslator::translate(Event::Key(release)), None);
    }

    #[test]
    fn mouse_events_translate() {
        use crossterm::event::{MouseButton as MB, MouseEventKind as MK};
        let event = Event::Mouse(CTMouse {
            kind: MK::Down(MB::Left),
            row: 12,
            column: 34,
            modifiers: crossterm::event::KeyModifiers::NONE,
        });
        assert_eq!(
            InputTranslator::translate(event),
            Some(UiEvent::Mouse(MouseEvent {
                kind: MouseKind::Click,
                button: MouseButton::Left,
                row: 12,
                col: 34,
            }))
        );
    }
}
