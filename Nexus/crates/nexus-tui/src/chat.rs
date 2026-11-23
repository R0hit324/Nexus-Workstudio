//! Chat panel: bounded history plus the composer.

use std::collections::VecDeque;

use super::MAX_CHAT_HISTORY;

/// Origin of a chat line (system vs user).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatLineKind {
    Message,
    Join,
    Leave,
    Notice,
}

/// One chat line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatLine {
    pub from: String,
    pub text: String,
    pub kind: ChatLineKind,
    /// Monotonic sequence for ordering after reconnects.
    pub seq: u64,
}

/// Chat state: bounded history, composer, and scrollback.
#[derive(Debug, Clone, Default)]
pub struct ChatState {
    pub lines: VecDeque<ChatLine>,
    pub input: String,
    pub composing: bool,
    /// Cursor within the composer.
    pub cursor: usize,
    /// Lines from the latest line when scrolled up.
    pub scrollback: usize,
    next_seq: u64,
    pub auto_scroll: bool,
}

impl ChatState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            lines: VecDeque::new(),
            input: String::new(),
            composing: false,
            cursor: 0,
            scrollback: 0,
            next_seq: 0,
            auto_scroll: true,
        }
    }

    /// Appends a chat line, bounding history.
    pub fn push(&mut self, from: impl Into<String>, text: impl Into<String>, kind: ChatLineKind) {
        self.lines.push_back(ChatLine {
            from: from.into(),
            text: text.into(),
            kind,
            seq: self.next_seq,
        });
        self.next_seq += 1;
        while self.lines.len() > MAX_CHAT_HISTORY {
            self.lines.pop_front();
        }
        self.scrollback = 0;
    }

    /// Inserts into the composer at the composer cursor.
    pub fn type_char(&mut self, c: char) {
        self.input.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// Deletes before the composer cursor.
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let (i, byte) = self.input[..self.cursor].char_indices().last().unwrap();
        self.input.remove(i);
        self.cursor -= byte.len_utf8();
    }

    /// Moves the composer cursor.
    pub fn move_cursor(&mut self, delta: isize) {
        let len = self.input.len();
        self.cursor = ((self.cursor as isize + delta).clamp(0, len as isize)) as usize;
    }

    /// Takes the composer text and clears it; returns the drafted message.
    #[must_use]
    pub fn take_draft(&mut self) -> String {
        let draft = std::mem::take(&mut self.input);
        self.cursor = 0;
        self.composing = false;
        draft
    }

    /// Visible lines when `height` lines of history are shown.
    #[must_use]
    pub fn view(&self, height: usize) -> Vec<&ChatLine> {
        let height = height.min(self.lines.len());
        if height == 0 {
            return Vec::new();
        }
        let end = self
            .lines
            .len()
            .saturating_sub(self.scrollback.min(self.lines.len().saturating_sub(height)));
        let start = end - height;
        self.lines.range(start..end).collect()
    }

    pub fn scroll(&mut self, delta: isize) {
        let max = self.lines.len().saturating_sub(1) as isize;
        self.scrollback = (self.scrollback as isize + delta).clamp(0, max) as usize;
        self.auto_scroll = self.scrollback == 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_is_bounded_and_recent() {
        let mut c = ChatState::new();
        for i in 0..(MAX_CHAT_HISTORY + 10) {
            c.push("u", i.to_string(), ChatLineKind::Message);
        }
        assert_eq!(c.lines.len(), MAX_CHAT_HISTORY);
        assert_eq!(c.lines.front().unwrap().text, "10");
    }

    #[test]
    fn composer_editing_works() {
        let mut c = ChatState::new();
        c.type_char('h');
        c.type_char('i');
        assert_eq!(c.input, "hi");
        c.move_cursor(-1);
        c.type_char('!');
        assert_eq!(c.input, "h!i");
        c.backspace();
        assert_eq!(c.input, "hi");
        c.move_cursor(-100);
        assert_eq!(c.cursor, 0);
    }

    #[test]
    fn take_draft_empties_composer() {
        let mut c = ChatState::new();
        c.composing = true;
        c.type_char('x');
        assert_eq!(c.take_draft(), "x");
        assert!(c.input.is_empty());
        assert!(!c.composing);
    }

    #[test]
    fn sequences_are_monotonic() {
        let mut c = ChatState::new();
        c.push("a", "1", ChatLineKind::Join);
        c.push("b", "2", ChatLineKind::Message);
        let seqs: Vec<u64> = c.lines.iter().map(|l| l.seq).collect();
        assert_eq!(seqs, vec![0, 1]);
    }

    #[test]
    fn scroll_releases_auto_scroll() {
        let mut c = ChatState::new();
        for i in 0..20 {
            c.push("u", i.to_string(), ChatLineKind::Message);
        }
        assert!(c.auto_scroll);
        c.scroll(1);
        assert!(!c.auto_scroll);
        c.scroll(-1);
        assert!(c.auto_scroll);
    }

    #[test]
    fn view_windows_latest_lines() {
        let mut c = ChatState::new();
        for i in 0..5 {
            c.push("u", i.to_string(), ChatLineKind::Message);
        }
        let visible = c.view(3);
        let texts: Vec<&str> = visible.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, vec!["2", "3", "4"]);
    }
}
