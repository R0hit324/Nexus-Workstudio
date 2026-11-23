//! Search (find) and replace within one document.
//!
//! Search state is a query plus the list of non-overlapping matches; the
//! active match can be cycled. Replace-all works from the end so earlier
//! offsets stay valid while the buffer shrinks.

use super::buffer::GapBuffer;

/// In-document search state.
#[derive(Debug, Clone, Default)]
pub struct SearchEngine {
    pub query: String,
    /// (start, end) logical offsets of every match.
    pub matches: Vec<(usize, usize)>,
    /// Index into `matches` of the active one.
    pub current: usize,
}

impl SearchEngine {
    /// Re-runs the search for `query`. Empty query clears state.
    pub fn set_query(&mut self, buffer: &GapBuffer, query: &str) {
        self.query = query.to_owned();
        self.matches = if query.is_empty() {
            Vec::new()
        } else {
            buffer.find(query)
        };
        self.current = 0;
    }

    #[must_use]
    pub fn active_match(&self) -> Option<(usize, usize)> {
        self.matches.get(self.current).copied()
    }

    #[must_use]
    pub fn is_active(&self) -> bool {
        !self.matches.is_empty()
    }

    /// Moves to the next/previous match.
    pub fn cycle(&mut self, delta: isize) -> Option<(usize, usize)> {
        match self.matches.len() {
            0 => None,
            n => {
                self.current = (self.current as isize + delta).rem_euclid(n as isize) as usize;
                self.active_match()
            }
        }
    }

    /// Offset the cursor should jump to for match `current`.
    #[must_use]
    pub fn focus_offset(&self) -> Option<usize> {
        self.active_match().map(|(start, _)| start)
    }

    /// Replaces all matches with `replacement`; returns how many changed.
    pub fn replace_all(&mut self, buffer: &mut GapBuffer, replacement: &str) -> usize {
        let matches = std::mem::take(&mut self.matches);
        let mut count = 0;
        for (start, end) in matches.iter().rev() {
            buffer.delete_range(*start, *end);
            buffer.insert_str(*start, replacement);
            count += 1;
        }
        self.matches.clear();
        self.current = 0;
        count
    }

    /// Replaces just the active match and re-searches.
    pub fn replace_current(&mut self, buffer: &mut GapBuffer, replacement: &str) -> bool {
        let Some((start, end)) = self.active_match() else {
            return false;
        };
        buffer.delete_range(start, end);
        buffer.insert_str(start, replacement);
        let target = start + replacement.chars().count();
        self.set_query(buffer, &self.query.clone());
        self.current = self
            .matches
            .iter()
            .position(|&(s, _)| s >= target)
            .unwrap_or(0);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(s: &str) -> GapBuffer {
        GapBuffer::from_str(s)
    }

    #[test]
    fn search_finds_and_cycles() {
        let buffer = buf("cat bat cat mat");
        let mut s = SearchEngine::default();
        s.set_query(&buffer, "cat");
        assert_eq!(s.matches, vec![(0, 3), (8, 11)]);
        assert_eq!(s.current, 0);
        assert_eq!(s.cycle(1), Some((8, 11)));
        assert_eq!(s.cycle(1), Some((0, 3)));
        assert_eq!(s.cycle(-1), Some((8, 11)));
    }

    #[test]
    fn empty_query_clears() {
        let buffer = buf("x");
        let mut s = SearchEngine::default();
        s.set_query(&buffer, "x");
        assert!(s.is_active());
        s.set_query(&buffer, "");
        assert!(!s.is_active());
        assert!(s.active_match().is_none());
    }

    #[test]
    fn no_hits_is_inactive() {
        let buffer = buf("aaa");
        let mut s = SearchEngine::default();
        s.set_query(&buffer, "b");
        assert!(!s.is_active());
        assert!(s.cycle(1).is_none());
    }

    #[test]
    fn replace_all_counts() {
        let mut buffer = buf("one one one");
        let mut s = SearchEngine::default();
        s.set_query(&buffer, "one");
        assert_eq!(s.replace_all(&mut buffer, "1"), 3);
        assert_eq!(buffer.to_string(), "1 1 1");
        assert!(!s.is_active());
    }

    #[test]
    fn replace_active_then_reseek() {
        let mut buffer = buf("a x b x");
        let mut s = SearchEngine::default();
        s.set_query(&buffer, "x");
        assert_eq!(s.current, 0);
        assert!(s.replace_current(&mut buffer, "Z"));
        assert_eq!(buffer.to_string(), "a Z b x");
        assert_eq!(s.matches.len(), 1);
        assert_eq!(s.matches[0], (6, 7));
    }
}
