//! Terminal emulation crate.
//!
//! Part 10 implements isolated Cube terminals, PTY management, and terminal rendering.

pub mod parser;
pub mod pty;
pub mod render;
pub mod session;

pub use parser::{AnsiEvent, AnsiParser, EraseMode};
pub use pty::{NativePtyBackend, PtyBackend, PtyConfig, Signal};
pub use render::{Attribute, Cell, Color, Cursor, CursorShape, RenderConfig, TerminalRenderer};
pub use session::{TerminalEvent, TerminalManager, TerminalSessionConfig};

/// Phase marker for version detection.
pub const PHASE: &str = "part10-terminal";

#[cfg(test)]
mod tests {
    #[test]
    fn phase_is_correct() {
        assert_eq!(super::PHASE, "part10-terminal");
    }
}
