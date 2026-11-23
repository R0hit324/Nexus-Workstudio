//! Cursor, selection, and multi-cursor sets.

use super::buffer::GapBuffer;

/// A vertical/horizontal caret position within a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    pub offset: usize,
    /// Preferred visual column kept across vertical moves.
    pub desired_col: Option<usize>,
}

impl Cursor {
    #[must_use]
    pub fn new(offset: usize) -> Self {
        Self {
            offset,
            desired_col: None,
        }
    }

    #[must_use]
    pub fn line(&self, buffer: &GapBuffer) -> usize {
        buffer.line_of(self.offset)
    }

    #[must_use]
    pub fn col(&self, buffer: &GapBuffer) -> usize {
        buffer.line_col(self.offset).1
    }

    pub fn move_left(&mut self, _buffer: &GapBuffer) -> bool {
        if self.offset > 0 {
            self.offset -= 1;
            true
        } else {
            false
        }
    }

    pub fn move_right(&mut self, buffer: &GapBuffer) -> bool {
        if self.offset < buffer.len() {
            self.offset += 1;
            true
        } else {
            false
        }
    }

    pub fn to_line_start(&mut self) {
        self.desired_col = None;
    }

    pub fn to_line_end(&mut self, buffer: &GapBuffer) {
        if let Some(end) = buffer.line_end(self.line(buffer)) {
            self.offset = end;
        }
        self.desired_col = None;
    }

    /// Moves vertically by `delta` lines, keeping the desired column.
    pub fn move_line(&mut self, buffer: &GapBuffer, delta: isize) -> bool {
        let line = self.line(buffer);
        let last = buffer.line_count().saturating_sub(1);
        let new_line = ((line as isize + delta).max(0) as usize).min(last);
        let desired = self.desired_col.unwrap_or_else(|| self.col(buffer));
        self.desired_col = Some(desired);
        if new_line == line {
            return false;
        }
        match buffer.line_start(new_line) {
            Some(start) => {
                let end = buffer.line_end(new_line).unwrap_or(start);
                let target = start + desired;
                self.offset = if target <= end { target } else { start };
                true
            }
            None => false,
        }
    }

    /// Moves to the end of the current word (the following boundary).
    pub fn next_word(&mut self, buffer: &GapBuffer) -> bool {
        let mut i = self.offset;
        let len = buffer.len();
        while i < len
            && buffer
                .char_at(i)
                .map(|c| c.is_alphanumeric())
                .unwrap_or(false)
        {
            i += 1;
        }
        let changed = i != self.offset;
        self.offset = i;
        changed
    }

    /// Moves to the last character of the word before the cursor.
    pub fn prev_word(&mut self, buffer: &GapBuffer) -> bool {
        let mut i = self.offset;
        while i > 0
            && buffer
                .char_at(i - 1)
                .map(|c| c.is_alphanumeric())
                .unwrap_or(false)
        {
            i -= 1;
        }
        while i > 0
            && buffer
                .char_at(i - 1)
                .map(|c| !c.is_alphanumeric())
                .unwrap_or(false)
        {
            i -= 1;
        }
        if i > 0
            && buffer
                .char_at(i - 1)
                .map(|c| c.is_alphanumeric())
                .unwrap_or(false)
        {
            i -= 1;
        }
        let changed = i != self.offset;
        self.offset = i;
        changed
    }
}

/// An optional selection driven by a primary cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    /// The anchor offset; `None` means no active selection.
    pub anchor: Option<usize>,
    pub active: usize,
}

impl Selection {
    #[must_use]
    pub const fn none_at(offset: usize) -> Self {
        Self {
            anchor: None,
            active: offset,
        }
    }

    /// Starts a new selection anchored at the previous active offset.
    pub fn start(direction: SelectionDirection, active: usize) -> Self {
        let anchor = match direction {
            SelectionDirection::Backward => active + 1,
            SelectionDirection::Forward => active.saturating_sub(1),
        };
        Self {
            anchor: Some(anchor),
            active,
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.begin() == self.end()
    }

    #[must_use]
    pub fn begin(&self) -> usize {
        match self.anchor {
            Some(a) => a.min(self.active),
            None => self.active,
        }
    }

    #[must_use]
    pub fn end(&self) -> usize {
        match self.anchor {
            Some(a) => a.max(self.active),
            None => self.active,
        }
    }

    #[must_use]
    pub fn range(&self) -> std::ops::Range<usize> {
        self.begin()..self.end()
    }

    /// The text currently selected, if any.
    #[must_use]
    pub fn selected_text<'a>(&self, buffer: &'a GapBuffer) -> Option<String> {
        if self.is_empty() {
            None
        } else {
            Some(buffer.slice(self.begin(), self.end()))
        }
    }
}

/// Which direction an extend-selection command moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionDirection {
    Forward,
    Backward,
}

/// A set of cursors (multi-cursor). Cursor 0 is the primary.
#[derive(Debug, Clone, Default)]
pub struct CursorSet {
    pub cursors: Vec<Cursor>,
    pub primary: usize,
}

impl CursorSet {
    #[must_use]
    pub fn single(offset: usize) -> Self {
        Self {
            cursors: vec![Cursor::new(offset)],
            primary: 0,
        }
    }

    #[must_use]
    pub fn primary_cursor(&self) -> &Cursor {
        &self.cursors[self.primary]
    }

    #[must_use]
    pub fn primary_cursor_mut(&mut self) -> &mut Cursor {
        &mut self.cursors[self.primary]
    }

    /// Adds a cursor if it isn't already present.
    pub fn add_cursor(&mut self, offset: usize) {
        if !self.cursors.iter().any(|c| c.offset == offset) {
            self.cursors.push(Cursor::new(offset));
        }
    }

    /// Resets to a single cursor at `offset`.
    pub fn reset(&mut self, offset: usize) {
        self.cursors.clear();
        self.cursors.push(Cursor::new(offset));
        self.primary = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::buffer::GapBuffer;

    fn text_cols() -> GapBuffer {
        GapBuffer::from_str("ab cd\nefg hijk\nxy")
    }

    #[test]
    fn horizontal_moves_clamp_to_buffer() {
        let buf = text_cols();
        let mut c = Cursor::new(0);
        assert!(!c.move_left(&buf));
        c.move_right(&buf);
        assert_eq!(c.offset, 1);
        let mut end = Cursor::new(buf.len());
        assert!(!end.move_right(&buf));
    }

    #[test]
    fn vertical_moves_preserve_column() {
        let buf = text_cols();
        let mut c = Cursor::new(3); // col 3 on "ab cd"
        assert!(c.move_line(&buf, 1));
        assert_eq!(c.line(&buf), 1);
        assert_eq!(c.offset, 9); // col 3 on line 1 -> "hijk" at 8..12
                                 // Going up preserves the desired column.
        c.move_line(&buf, -1);
        assert_eq!(c.line(&buf), 0);
        assert_eq!(c.offset, 3);
        // Past the bottom clamps to the last line.
        c.move_line(&buf, 5);
        assert_eq!(c.line(&buf), 2);
        assert_eq!(c.offset, 15);
    }

    #[test]
    fn word_moves_boundary_based() {
        let buf = text_cols();
        let mut c = Cursor::new(0);
        c.next_word(&buf);
        assert_eq!(c.offset, 2); // "ab cd" -> "ab" then space at 2? next_word lands at space? expects "cd"
        let mut c2 = Cursor::new(buf.len());
        c2.prev_word(&buf);
        assert_eq!(c2.offset, 13);
    }

    #[test]
    fn selection_ranges() {
        let buf = text_cols();
        let mut sel = Selection::none_at(2);
        sel.anchor = Some(5);
        sel.active = 3;
        assert_eq!(sel.range(), 3..5);
        assert_eq!(sel.selected_text(&buf).unwrap(), "cd");
        // Empty selection has no text.
        let empty = Selection::none_at(3);
        assert!(empty.selected_text(&buf).is_none());
    }

    #[test]
    fn cursor_set_adds_unique() {
        let mut set = CursorSet::single(0);
        set.add_cursor(4);
        set.add_cursor(4);
        set.add_cursor(1);
        assert_eq!(set.cursors.len(), 3);
        set.reset(9);
        assert_eq!(set.cursors.len(), 1);
        assert_eq!(set.primary_cursor().offset, 9);
    }
}
