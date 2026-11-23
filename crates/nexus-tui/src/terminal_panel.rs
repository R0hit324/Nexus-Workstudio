//! Terminal panel state: bounded output scrollback for running commands.
//!
//! Output arrives asynchronously from the session service and is appended
//! here; the panel only renders the currently visible window.

use std::collections::VecDeque;

use super::MAX_TERMINAL_SCROLLBACK;

/// A line of terminal output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalLine {
    pub text: String,
    pub stderr: bool,
}

/// Scrollable, bounded terminal output.
#[derive(Debug, Clone, Default)]
pub struct TerminalPanelState {
    pub lines: VecDeque<TerminalLine>,
    /// Cursor offset from the bottom (0 = newest visible).
    pub scrollback: usize,
    /// True while a command is running.
    pub running: bool,
    /// Exit code of the last finished command, if any.
    pub last_exit: Option<i32>,
}

impl TerminalPanelState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            lines: VecDeque::new(),
            scrollback: 0,
            running: false,
            last_exit: None,
        }
    }

    /// Appends a line, keeping the queue bounded.
    pub fn push(&mut self, text: impl Into<String>, stderr: bool) {
        let mut text = text.into();
        if text.ends_with('\n') {
            text.pop();
        }
        if text.is_empty() {
            return;
        }
        self.lines.push_back(TerminalLine { text, stderr });
        while self.lines.len() > MAX_TERMINAL_SCROLLBACK {
            self.lines.pop_front();
        }
    }

    /// Pushes raw PTY output bytes, splitting on newlines and treating the
    /// final unterminated fragment as the start of the next line.
    ///
    /// ANSI escape sequences are passed through verbatim here; the renderer
    /// (which handles colors/clears) consumes them per line on draw.
    pub fn push_bytes(&mut self, data: &[u8]) {
        let text = String::from_utf8_lossy(data);
        for line in text.split('\n') {
            // Preserve the trailing piece of an unterminated line by pushing
            // empty fragments as line breaks instead of blank rendering lines.
            let fragment = line.trim_end_matches('\r');
            self.push(fragment.to_string(), false);
        }
        // `push` drops empty lines; that is desired between split lines.
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.scrollback = 0;
    }

    /// The slice of lines currently visible when `height` lines are shown.
    #[must_use]
    pub fn view(&self, height: usize) -> Vec<&TerminalLine> {
        let height = height.min(self.lines.len());
        if height == 0 {
            return Vec::new();
        }
        // `scrollback` counts from the newest line; the window cannot start
        // before index 0.
        let end = self
            .lines
            .len()
            .saturating_sub(self.scrollback.min(self.lines.len().saturating_sub(height)));
        let start = end - height;
        self.lines.range(start..end).collect()
    }

    /// Paging up/down within the scrollback.
    pub fn scroll(&mut self, delta: isize) {
        let max = self.lines.len().saturating_sub(1) as isize;
        self.scrollback = (self.scrollback as isize + delta).clamp(0, max) as usize;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_trims_lines_and_normalizes_newline() {
        let mut t = TerminalPanelState::new();
        t.push("a\n", false);
        t.push("b", true);
        assert_eq!(t.lines.len(), 2);
        assert_eq!(t.lines[0].text, "a");
        assert!(t.lines[1].stderr);
    }

    #[test]
    fn empty_lines_are_dropped() {
        let mut t = TerminalPanelState::new();
        t.push("\n", false);
        assert!(t.lines.is_empty());
    }

    #[test]
    fn queue_is_bounded() {
        let mut t = TerminalPanelState::new();
        for i in 0..(MAX_TERMINAL_SCROLLBACK + 50) {
            t.push(i.to_string(), false);
        }
        assert_eq!(t.lines.len(), MAX_TERMINAL_SCROLLBACK);
        assert_eq!(t.lines.front().unwrap().text, "50");
    }

    #[test]
    fn scroll_clamps() {
        let mut t = TerminalPanelState::new();
        for i in 0..10 {
            t.push(i.to_string(), false);
        }
        t.scroll(100);
        assert_eq!(t.scrollback, 9);
        t.scroll(-100);
        assert_eq!(t.scrollback, 0);
        t.clear();
        assert!(t.lines.is_empty());
    }

    #[test]
    fn running_flags() {
        let mut t = TerminalPanelState::new();
        assert!(!t.running);
        t.running = true;
        t.last_exit = Some(0);
        assert_eq!(t.last_exit, Some(0));
    }

    #[test]
    fn push_bytes_splits_lines() {
        let mut t = TerminalPanelState::new();
        t.push_bytes(b"hello world\nsecond line\n");
        assert_eq!(t.lines.len(), 2);
        assert_eq!(t.lines[0].text, "hello world");
        assert_eq!(t.lines[1].text, "second line");

        t.push_bytes(b"no trailing newline");
        assert_eq!(t.lines[2].text, "no trailing newline");
    }

    #[test]
    fn push_bytes_strips_carriage_returns() {
        let mut t = TerminalPanelState::new();
        t.push_bytes(b"progress\r\nfinal\r\n");
        assert_eq!(t.lines[0].text, "progress");
        assert_eq!(t.lines[1].text, "final");
    }
}
