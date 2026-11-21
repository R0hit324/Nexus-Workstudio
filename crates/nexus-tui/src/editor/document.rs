//! One open document: buffer, cursor/selection, undo, search, metadata.

use std::path::PathBuf;

use nexus_core::ids::FileId;

use super::buffer::GapBuffer;
use super::cursor::{Cursor, Selection};
use super::search::SearchEngine;
use super::syntax::LanguageId;
use super::undo::{Edit, Origin, UndoManager};

/// An opened document in the editor.
#[derive(Debug, Clone)]
pub struct Document {
    pub file_id: FileId,
    pub path: PathBuf,
    pub buffer: GapBuffer,
    /// Monotonic version; bumped on every local or applied edit.
    pub version: u64,
    pub language: LanguageId,
    pub large: bool,
    /// Local edits not yet persisted/synced.
    pub dirty: bool,
    pub cursor: Cursor,
    pub selection: Selection,
    pub undo: UndoManager,
    pub search: SearchEngine,
}

impl Document {
    /// Opens `path` with `text`. `large` marks files above the display
    /// threshold (renderer skips syntax highlighting for them).
    #[must_use]
    pub fn open(file_id: FileId, path: PathBuf, text: &str, large: bool) -> Self {
        let language = LanguageId::from_path(&path);
        let buffer = GapBuffer::from_str(text);
        Self {
            file_id,
            path,
            version: 0,
            language,
            large,
            dirty: false,
            cursor: Cursor::new(0),
            selection: Selection::none_at(0),
            undo: UndoManager::with_limit(1024),
            search: SearchEngine::default(),
            buffer,
        }
    }

    /// Inserts `text` at the cursor, recording undo history.
    pub fn insert_at_cursor(&mut self, text: &str) {
        let chars: Vec<char> = text.chars().collect();
        if chars.is_empty() {
            return;
        }
        let pos = self.cursor.offset;
        self.undo.record(pos, &chars, &[], Origin::Local);
        self.buffer.insert_str(pos, text);
        self.cursor.offset = pos + chars.len();
        self.bump_dirty();
    }

    /// Deletes the character before the cursor.
    pub fn delete_backward(&mut self) {
        if self.delete_selection_present() {
            self.delete_selection();
            return;
        }
        if self.cursor.offset == 0 {
            return;
        }
        let pos = self.cursor.offset - 1;
        if let Some(removed) = self.buffer.delete_backward(self.cursor.offset) {
            self.undo.record(pos, &[], &[removed], Origin::Local);
            self.cursor.offset = pos;
            self.bump_dirty();
        }
    }

    /// Deletes the character at the cursor. At end of buffer this falls back
    /// to deleting the character behind the cursor.
    pub fn delete_forward(&mut self) {
        if self.delete_selection_present() {
            self.delete_selection();
            return;
        }
        if self.cursor.offset >= self.buffer.len() {
            if self.cursor.offset == 0 {
                return;
            }
            self.delete_backward();
            return;
        }
        if let Some(removed) = self.buffer.delete_forward(self.cursor.offset) {
            self.undo
                .record(self.cursor.offset, &[], &[removed], Origin::Local);
            self.bump_dirty();
        }
    }

    /// Whether the current selection is non-empty (and therefore active).
    #[must_use]
    pub fn has_selection(&self) -> bool {
        !self.selection.is_empty()
    }

    /// Deletes the selected range and collapses the caret at its start.
    pub fn delete_selection(&mut self) {
        let range = self.selection.range();
        if range.start == range.end {
            return;
        }
        if let Some(removed) = self.buffer.delete_range(range.start, range.end) {
            self.undo.record(range.start, &[], &removed, Origin::Local);
            self.cursor.offset = range.start;
            self.selection = Selection::none_at(range.start);
            self.bump_dirty();
        }
    }

    /// Cuts the selection (copy + delete).
    #[must_use]
    pub fn cut(&mut self) -> Option<String> {
        let text = self.selection.selected_text(&self.buffer)?;
        self.delete_selection();
        Some(text)
    }

    /// Copies the selection without modifying the buffer.
    #[must_use]
    pub fn copy(&self) -> Option<String> {
        self.selection.selected_text(&self.buffer)
    }

    /// Pastes `text` at the cursor.
    pub fn paste(&mut self, text: &str) {
        self.insert_at_cursor(text);
    }

    /// Commits any pending typing burst, making undo steps stable.
    pub fn commit_undo(&mut self) {
        self.undo.commit();
    }

    pub fn undo(&mut self) -> bool {
        if let Some(edit) = self.undo.undo() {
            self.apply_edit(edit);
            return true;
        }
        false
    }

    pub fn redo(&mut self) -> bool {
        if let Some(edit) = self.undo.redo() {
            self.apply_edit(edit);
            return true;
        }
        false
    }

    /// Applies an edit from the undo system to the buffer.
    fn apply_edit(&mut self, edit: Edit) {
        self.buffer
            .delete_range(edit.pos, edit.pos + edit.removed.len());
        let inserted: String = edit.inserted.iter().collect();
        self.buffer.insert_str(edit.pos, &inserted);
        self.cursor.offset = edit.pos + edit.inserted.len();
        self.selection = Selection::none_at(self.cursor.offset);
        self.bump_dirty();
    }

    /// Replaces all search matches and clears unrelated undo history.
    pub fn replace_all(&mut self, replacement: &str) -> usize {
        let count = self.search.replace_all(&mut self.buffer, replacement);
        if count > 0 {
            self.undo.clear();
            self.bump_dirty();
        }
        count
    }

    /// Full UTF-8 text, for persistence and network sync.
    #[must_use]
    pub fn fingerprint(&self) -> &GapBuffer {
        &self.buffer
    }

    #[must_use]
    pub fn line_count(&self) -> usize {
        self.buffer.line_count()
    }

    /// Clears the dirty flag after a successful save.
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    fn delete_selection_present(&self) -> bool {
        !self.selection.is_empty()
    }

    fn bump_dirty(&mut self) {
        self.dirty = true;
        self.version += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(text: &str) -> Document {
        Document::open(FileId::from_u64(1), PathBuf::from("doc.rs"), text, false)
    }

    #[test]
    fn typing_and_backspace_roundtrip() {
        let mut doc = open("");
        doc.insert_at_cursor("hel");
        doc.insert_at_cursor("lo");
        assert_eq!(doc.buffer.to_string(), "hello");
        doc.delete_backward();
        assert_eq!(doc.buffer.to_string(), "hell");
        doc.delete_forward();
        assert_eq!(doc.buffer.to_string(), "hel");
        doc.delete_backward();
        assert_eq!(doc.buffer.to_string(), "he");
        assert!(doc.dirty);
    }

    #[test]
    fn cursor_moves_mid_buffer_on_delete() {
        let mut doc = open("abcdef");
        doc.cursor.offset = 4;
        doc.delete_backward();
        assert_eq!(doc.buffer.to_string(), "abcef");
        assert_eq!(doc.cursor.offset, 3);
    }

    #[test]
    fn selection_delete_cuts_cleanly() {
        let mut doc = open("hello world");
        doc.cursor.offset = 6;
        doc.selection.active = 6;
        doc.selection.anchor = Some(11);
        assert_eq!(doc.copy().unwrap(), "world");
        assert_eq!(doc.cut().unwrap(), "world");
        assert_eq!(doc.buffer.to_string(), "hello ");
        assert!(!doc.has_selection());
        assert_eq!(doc.cursor.offset, 6);
    }

    #[test]
    fn cut_with_no_selection_is_none() {
        let mut doc = open("hi");
        assert!(doc.cut().is_none());
        assert_eq!(doc.buffer.to_string(), "hi");
    }

    #[test]
    fn undo_redo_restores_text_and_cursor() {
        let mut doc = open("abc");
        doc.cursor.offset = 3;
        doc.insert_at_cursor("XYZ");
        assert_eq!(doc.buffer.to_string(), "abcXYZ");
        assert!(doc.undo());
        assert_eq!(doc.buffer.to_string(), "abc");
        assert_eq!(doc.cursor.offset, 3);
        assert!(doc.redo());
        assert_eq!(doc.buffer.to_string(), "abcXYZ");
        assert_eq!(doc.cursor.offset, 6);
        // One more undo restores the pre-typing state.
        assert!(doc.undo());
        assert_eq!(doc.buffer.to_string(), "abc");
    }

    #[test]
    fn typing_burst_undoes_as_one_step() {
        let mut doc = open("");
        doc.insert_at_cursor("aaa");
        doc.insert_at_cursor("bb");
        doc.commit_undo();
        assert!(doc.undo());
        assert_eq!(doc.buffer.to_string(), "");
        assert!(!doc.undo());
    }

    #[test]
    fn replace_all_clears_history_and_counts() {
        let mut doc = open("x x x");
        doc.search.set_query(&doc.buffer, "x");
        assert_eq!(doc.replace_all("y"), 3);
        assert_eq!(doc.buffer.to_string(), "y y y");
        assert!(!doc.undo());
    }

    #[test]
    fn mark_clean_then_edit_dirty_again() {
        let mut doc = open("a");
        doc.mark_clean();
        assert!(!doc.dirty);
        doc.insert_at_cursor("b");
        assert!(doc.dirty);
    }

    #[test]
    fn version_is_monotonic() {
        let mut doc = open("a");
        let v0 = doc.version;
        doc.insert_at_cursor("b");
        assert!(doc.version > v0);
    }
}
