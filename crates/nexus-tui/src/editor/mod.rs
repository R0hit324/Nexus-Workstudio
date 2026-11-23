//! The editor: a terminal-free text engine with an eye toward collaboration.
//!
//! Modules:
//! - `buffer`  — gap-buffer storage (see `docs/decisions/ADR-006`).
//! - `cursor`  — caret, selection, multi-cursor sets.
//! - `document`— an opened file binding buffer, undo, search, and cursor.
//! - `undo`    — origin-tagged edit history ready for remote merging.
//! - `search`  — find/replace over a buffer.
//! - `syntax`  — lightweight highlighting (linear, no regex).
//! - `remote`  — other users' carets inside open documents.
//!
//! Nothing here touches the terminal, so every piece is unit-testable and
//! reusable headlessly (integration tests can edit documents without TUI).

pub mod buffer;
pub mod cursor;
pub mod document;
pub mod remote;
pub mod search;
pub mod syntax;
pub mod undo;

pub use buffer::GapBuffer;
pub use cursor::{Cursor, CursorSet, Selection};
pub use document::Document;
pub use remote::{RemoteCursor, RemoteCursorStore};
pub use search::SearchEngine;
pub use syntax::{HighlightKind, HighlightSpan, LanguageId, SyntaxProvider};
pub use undo::{Edit, Origin, UndoManager};
