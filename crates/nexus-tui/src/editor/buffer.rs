//! Gap buffer text storage.
//!
//! Chosen over a `String`/`Vec` for O(1) amortized insert/delete near the
//! cursor with no per-character allocation (see `docs/decisions/ADR-006`).
//! Editing may therefore handle large files without re-allocation on every
//! keystroke. Positions throughout the editor are *character* offsets into the
//! logical buffer.

/// A gap buffer over the text of one document.
#[derive(Debug, Clone)]
pub struct GapBuffer {
    buf: Vec<char>,
    gap_start: usize,
    gap_end: usize,
    len: usize,
    line_dirty: bool,
    line_starts: Vec<usize>,
}

impl GapBuffer {
    /// A buffer containing `text`.
    #[must_use]
    pub fn from_str(text: &str) -> Self {
        let chars: Vec<char> = text.chars().collect();
        let len = chars.len();
        // Place the initial gap at the end: the common case (typing at the
        // end of a file) then never triggers a gap move.
        let mut this = Self {
            buf: chars,
            gap_start: len,
            gap_end: len,
            len,
            line_dirty: true,
            line_starts: Vec::new(),
        };
        this.ensure_gap(32);
        this
    }

    /// An empty buffer.
    #[must_use]
    pub fn new() -> Self {
        Self::from_str("")
    }

    /// Logical length in characters.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Concatenates the logical text.
    #[must_use]
    pub fn to_string(&self) -> String {
        let mut out = String::with_capacity(self.len);
        out.extend(self.buf[..self.gap_start].iter());
        out.extend(self.buf[self.gap_end..].iter());
        out
    }

    /// Slices a logical range as a string.
    #[must_use]
    pub fn slice(&self, start: usize, end: usize) -> String {
        let chars = self.range_chars(start, end);
        chars.into_iter().collect()
    }

    /// Collects the characters in a logical range.
    pub fn range_chars(&self, start: usize, end: usize) -> Vec<char> {
        let start = start.min(self.len);
        let end = end.clamp(start, self.len);
        let mut out = Vec::with_capacity(end - start);
        for (logical, i) in Self::positions(self) {
            if logical >= end {
                break;
            }
            if logical >= start {
                out.push(i);
            }
        }
        out
    }

    /// Character at a logical offset.
    #[must_use]
    pub fn char_at(&self, offset: usize) -> Option<char> {
        let mut out = None;
        for (logical, ch) in Self::positions(self) {
            if logical == offset {
                out = Some(ch);
                break;
            }
            if logical > offset {
                break;
            }
        }
        out
    }

    /// Inserts `text` at `offset`.
    pub fn insert_str(&mut self, offset: usize, text: &str) {
        let mut at = offset.min(self.len);
        for ch in text.chars() {
            self.insert_char(at, ch);
            at += 1;
        }
    }

    /// Inserts a single character at `offset`.
    pub fn insert_char(&mut self, offset: usize, ch: char) {
        let offset = offset.min(self.len);
        self.move_gap_to(offset);
        self.ensure_gap(1);
        self.buf[self.gap_start] = ch;
        self.gap_start += 1;
        self.len += 1;
        if ch == '\n' {
            self.line_dirty = true;
        }
    }

    /// Deletes `[start, end)` and returns the removed characters.
    pub fn delete_range(&mut self, start: usize, end: usize) -> Option<Vec<char>> {
        if !(start < end && end <= self.len) {
            return None;
        }
        self.move_gap_to(start);
        let removed_len = end - start;
        let tail_start = self.gap_end + removed_len;
        let removed: Vec<char> = self.buf[self.gap_end..tail_start].to_vec();

        // Compact the trailing region down over the removed characters. The
        // gap's right edge (`gap_end`) stays fixed: everything before it is
        // kept, everything from it onward is the trimmed tail.
        let tail_len = self.buf.len() - tail_start;
        if tail_len > 0 {
            self.buf
                .copy_within(tail_start..tail_start + tail_len, self.gap_end);
        }
        self.buf.truncate(self.gap_end + tail_len);
        self.len -= removed_len;
        if removed.iter().any(|c| *c == '\n') {
            self.line_dirty = true;
        }
        Some(removed)
    }

    /// Deletes the character just before `offset`.
    pub fn delete_backward(&mut self, offset: usize) -> Option<char> {
        if offset == 0 {
            return None;
        }
        self.delete_range(offset - 1, offset)
            .map(|mut v| v.pop().unwrap())
    }

    /// Deletes the character at `offset`.
    pub fn delete_forward(&mut self, offset: usize) -> Option<char> {
        self.delete_range(offset, offset + 1)
            .map(|mut v| v.remove(0))
    }

    /// Replaces the whole logical content (load/reload).
    pub fn load(&mut self, text: &str) {
        let chars: Vec<char> = text.chars().collect();
        self.buf = chars;
        self.gap_start = 0;
        self.gap_end = 0;
        self.len = self.buf.len();
        self.line_dirty = true;
        self.ensure_gap(32);
    }

    /// The line/column (both 0-based) of a logical offset.
    #[must_use]
    pub fn line_col(&self, offset: usize) -> (usize, usize) {
        self.with_index(|starts| {
            let line = starts.partition_point(|&s| s <= offset).saturating_sub(1);
            let col = offset - starts[line];
            (line, col)
        })
    }

    /// Line at an offset (0-based). Equivalent to `line_col(offset).0`.
    #[must_use]
    pub fn line_of(&self, offset: usize) -> usize {
        self.line_col(offset).0
    }

    /// Character offset of the start of `line`.
    #[must_use]
    pub fn line_start(&self, line: usize) -> Option<usize> {
        self.with_index(|starts| starts.get(line).copied())
    }

    /// Character offset one past the last char of `line` (excludes newline).
    #[must_use]
    pub fn line_end(&self, line: usize) -> Option<usize> {
        let start = self.line_start(line)?;
        let mut end = start;
        for (logical, ch) in Self::positions(self) {
            if logical < start {
                continue;
            }
            if ch == '\n' {
                break;
            }
            end = logical + 1;
        }
        Some(end)
    }

    #[must_use]
    pub fn line_count(&self) -> usize {
        self.with_index(|starts| {
            let n = starts.len();
            if self.is_empty() {
                return 1;
            }
            n
        })
    }

    /// Characters that belong to line `line` (no trailing newline).
    #[must_use]
    pub fn line_text(&self, line: usize) -> String {
        match (self.line_start(line), self.line_end(line)) {
            (Some(s), Some(e)) => self.slice(s, e),
            _ => String::new(),
        }
    }

    /// Finds all non-overlapping occurrences of `needle` as (start, end) in
    /// logical offsets. Naive scan, O(n·m); fine for typed-size queries.
    #[must_use]
    pub fn find(&self, needle: &str) -> Vec<(usize, usize)> {
        let needle: Vec<char> = needle.chars().collect();
        if needle.is_empty() {
            return Vec::new();
        }
        let positions: Vec<(usize, char)> = Self::positions(self).collect();
        if needle.len() > positions.len() {
            return Vec::new();
        }
        let mut hits = Vec::new();
        for start in 0..=positions.len() - needle.len() {
            if positions[start..start + needle.len()]
                .iter()
                .zip(&needle)
                .all(|((_, c), n)| c == n)
            {
                hits.push((
                    positions[start].0,
                    positions[start + needle.len() - 1].0 + 1,
                ));
            }
        }
        hits
    }

    // --- Internals -------------------------------------------------------

    /// Iterates logical positions with their characters, skipping the gap.
    fn positions(&self) -> impl Iterator<Item = (usize, char)> + '_ {
        let gap_start = self.gap_start;
        let gap_end = self.gap_end;
        self.buf
            .iter()
            .enumerate()
            .filter_map(move |(physical, &ch)| {
                let logical = if physical < gap_start {
                    physical
                } else if physical >= gap_end {
                    physical - (gap_end - gap_start)
                } else {
                    return None;
                };
                Some((logical, ch))
            })
    }

    fn move_gap_to(&mut self, offset: usize) {
        if offset == self.gap_start {
            return;
        }
        if offset < self.gap_start {
            // Shift the left block right, adjacent to the gap's right edge.
            let count = self.gap_start - offset;
            let dest_start = self.gap_end - count;
            self.buf.copy_within(offset..self.gap_start, dest_start);
            self.gap_start = offset;
            self.gap_end = dest_start;
        } else {
            // Shift the right block left into the gap.
            let count = offset - self.gap_start;
            let src_start = self.gap_end;
            self.buf
                .copy_within(src_start..src_start + count, self.gap_start);
            self.gap_start += count;
            self.gap_end += count;
        }
    }

    fn ensure_gap(&mut self, extra: usize) {
        let current = self.gap_end - self.gap_start;
        if current >= extra {
            return;
        }
        let target = (self.len / 2 + extra).max(32);
        let grow = target - current;
        let old_len = self.buf.len();
        self.buf.resize(old_len + grow, '\0');
        if self.gap_end < old_len {
            self.buf
                .copy_within(self.gap_end..old_len, self.gap_end + grow);
        }
        self.gap_end += grow;
    }

    /// Runs `f` over the current line-start index, rebuilding it on the fly
    /// when a mutation has dirtied the cached index. Read-only in spirit, so
    /// callers holding `&self` can query line geometry.
    fn with_index<T>(&self, f: impl FnOnce(&[usize]) -> T) -> T {
        if !self.line_dirty {
            return f(&self.line_starts);
        }
        let mut starts = vec![0usize];
        for (logical, ch) in Self::positions(self) {
            if ch == '\n' {
                starts.push(logical + 1);
            }
        }
        f(&starts)
    }
}

impl Default for GapBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(s: &str) -> GapBuffer {
        GapBuffer::from_str(s)
    }

    #[test]
    fn empty_buffer_roundtrips() {
        let buf = GapBuffer::new();
        assert!(buf.is_empty());
        assert_eq!(buf.to_string(), "");
        assert_eq!(buf.line_count(), 1);
    }

    #[test]
    fn load_and_string_roundtrip() {
        let text = "fn main() {\n    println!(\"hi\");\n}\n";
        let buf = load(text);
        assert_eq!(buf.to_string(), text);
        assert_eq!(buf.len(), text.chars().count());
    }

    #[test]
    fn insert_at_front_middle_end() {
        let mut buf = load("abc");
        buf.insert_str(0, "X");
        assert_eq!(buf.to_string(), "Xabc");
        buf.insert_str(2, "YZ");
        assert_eq!(buf.to_string(), "XaYZbc");
        buf.insert_str(buf.len(), "!");
        assert_eq!(buf.to_string(), "XaYZbc!");
    }

    #[test]
    fn typed_characters_land_in_order() {
        let mut buf = GapBuffer::new();
        for (i, c) in "hello".chars().enumerate() {
            buf.insert_char(i, c);
        }
        for (i, c) in "hello".chars().enumerate() {
            assert_eq!(buf.char_at(i), Some(c));
        }
        assert_eq!(buf.to_string(), "hello");
    }

    #[test]
    fn delete_ranges_and_backwards() {
        let mut buf = load("abcdef");
        assert_eq!(buf.delete_range(1, 4), Some(vec!['b', 'c', 'd']));
        assert_eq!(buf.to_string(), "aef");
        assert_eq!(buf.delete_backward(2), Some('e'));
        assert_eq!(buf.to_string(), "af");
        assert_eq!(buf.delete_backward(0), None);
        assert_eq!(buf.delete_forward(1), Some('f'));
        assert_eq!(buf.to_string(), "a");
        assert_eq!(buf.delete_range(5, 5), None);
    }

    #[test]
    fn insert_between_moves_gap_repeatedly() {
        // Stress gap movement: type at the end, jump to front, back to end.
        let mut buf = load("xy");
        buf.insert_str(0, "A");
        assert_eq!(buf.to_string(), "Axy");
        buf.insert_str(buf.len(), "Z");
        assert_eq!(buf.to_string(), "AxyZ");
        buf.insert_str(1, "B");
        assert_eq!(buf.to_string(), "ABxyZ");
        buf.insert_str(5, "Q");
        assert_eq!(buf.to_string(), "ABxyZQ");
    }

    #[test]
    fn line_index_is_accurate() {
        let buf = load("ab\ncd\nef\n");
        assert_eq!(buf.line_count(), 4); // trailing newline => 4 lines
        assert_eq!(buf.line_start(0), Some(0));
        assert_eq!(buf.line_start(1), Some(3));
        assert_eq!(buf.line_start(2), Some(6));
        assert_eq!(buf.line_start(3), Some(9));
        assert_eq!(buf.line_col(0), (0, 0));
        assert_eq!(buf.line_col(4), (1, 1));
        assert_eq!(buf.line_col(10), (3, 1));
    }

    #[test]
    fn line_text_respects_line_boundaries() {
        let buf = load("row0\nrow1");
        assert_eq!(buf.line_text(0), "row0");
        assert_eq!(buf.line_text(1), "row1");
    }

    #[test]
    fn line_index_updates_after_edits() {
        let mut buf = load("ab\ncd");
        buf.insert_str(2, "X\nY"); // "abX\nY\ncd"
        assert_eq!(buf.to_string(), "abX\nY\ncd");
        assert_eq!(buf.line_col(2), (0, 2));
        assert_eq!(buf.line_count(), 3);
        assert_eq!(buf.line_start(2), Some(6));
    }

    #[test]
    fn find_locates_matches() {
        let buf = load("banana appellate ban");
        let hits = buf.find("ban");
        assert_eq!(hits, vec![(0, 3), (17, 20)]);
        assert!(buf.find("nope").is_empty());
        assert!(buf.find("").is_empty());
    }

    #[test]
    fn slice_and_range_chars_are_consistent() {
        let buf = load("αβγ δε");
        assert_eq!(buf.slice(1, 4), "βγ ");
        assert_eq!(buf.range_chars(1, 3), vec!['β', 'γ']);
    }

    #[test]
    fn load_replaces_content() {
        let mut buf = load("old content");
        buf.load("new");
        assert_eq!(buf.to_string(), "new");
        assert_eq!(buf.line_count(), 1);
    }

    #[test]
    fn large_edits_do_not_corrupt() {
        let mut buf = GapBuffer::new();
        let base = "some line\n";
        for _ in 0..2000 {
            for c in base.chars() {
                let at = buf.len();
                buf.insert_char(at, c);
            }
        }
        assert_eq!(buf.line_count(), 2001);
        assert_eq!(buf.slice(0, "some line\n".len()), "some line\n");
    }
}
