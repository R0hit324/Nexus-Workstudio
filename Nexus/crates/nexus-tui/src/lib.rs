//! `nexus-tui` is the terminal user interface for the Nexus client.
//!
//! Architecture layers (see `docs/architecture/overview.md`):
//!
//! ```text
//! TUI (panels, widgets)          -- ratatui, thin, terminal-only
//! Application (App, state, loop) -- owns domain state and command dispatch
//! Client services (nexus-client) -- session/workspace/events/network/cache
//! Nexus protocol + transport     -- never touched by widgets
//! ```
//!
//! Widgets never talk to the network: every transition flows through the
//! [`Action`] / [`Command`] system and the application event loop.

pub mod actions;
pub mod app;
pub mod build_panel;
pub mod chat;
pub mod commands;
pub mod dialogs;
pub mod editor;
pub mod explorer;
pub mod input;
pub mod keymap;
pub mod layout;
pub mod notifications;
pub mod palette;
pub mod panels;
pub mod persistence;
pub mod splits;
pub mod status;
pub mod tabs;
pub mod terminal_panel;
pub mod theme;
pub mod users;

pub use actions::Action;
pub use commands::{Command, CommandRegistry};
pub use input::{InputTranslator, Key, MouseButton, MouseEvent, MouseKind, UiEvent};
pub use keymap::{Binding, Keymap, Mode};
pub use layout::{PanelId, PanelRuntime, Rect};

/// Maximum number of editor documents kept open (bounds memory).
pub const MAX_OPEN_DOCUMENTS: usize = 64;
/// Maximum chat lines retained in the panel (bounded history).
pub const MAX_CHAT_HISTORY: usize = 512;
/// Maximum terminal scrollback lines retained.
pub const MAX_TERMINAL_SCROLLBACK: usize = 10_000;
/// Maximum queued notifications.
pub const MAX_NOTIFICATIONS: usize = 64;
/// Files larger than this are treated as "large" and get light processing.
pub const LARGE_FILE_THRESHOLD: usize = 5 * 1024 * 1024;
