//! Undo/redo with origin tracking for later collaborative merging.
//!
//! Every editing action is recorded as an `Edit` stamped with its `Origin`.
//! Local edits are grouped into a single undo unit (typing bursts collapse
//! into one step); remote edits (later parts) stay separate so collaborative
//! history can be replay-merged.

/// Who caused an edit. Used by the collaboration engine (later parts) to
/// decide whether a unit can be undone wholesale for one user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Local,
    Remote,
    Programmatic,
}

/// A single logical edit: what was removed and what replaced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub pos: usize,
    pub inserted: Vec<char>,
    pub removed: Vec<char>,
    pub origin: Origin,
}

impl Edit {
    #[must_use]
    pub fn new(pos: usize, inserted: Vec<char>, removed: Vec<char>, origin: Origin) -> Self {
        Self {
            pos,
            inserted,
            removed,
            origin,
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inserted.is_empty() && self.removed.is_empty()
    }
}

/// Bounded undo/redo stacks. Group pushes the current typed burst into one unit.
#[derive(Debug, Clone)]
pub struct UndoManager {
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    pending: Option<Edit>,
    limit: usize,
}

impl UndoManager {
    /// Creates a manager holding at most `limit` undo units.
    #[must_use]
    pub fn with_limit(limit: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            pending: None,
            limit,
        }
    }

    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.undo.is_empty() && self.pending.is_none()
    }

    /// Records a completed edit at cursor position `pos`. Consecutive local
    /// insertions (a typing burst) fold into a single undo unit regardless of
    /// key; any deletion or non-local edit starts a fresh unit.
    pub fn record(&mut self, pos: usize, inserted: &[char], removed: &[char], origin: Origin) {
        let Some(_) = inserted.iter().chain(removed.iter()).next().copied() else {
            return;
        };
        let mergeable = match self.pending.as_ref() {
            Some(e) => {
                e.origin == origin
                    && origin == Origin::Local
                    && !e.inserted.is_empty()
                    && !inserted.is_empty()
            }
            None => false,
        };

        if mergeable {
            let unit = self.pending.as_mut().unwrap();
            unit.inserted.extend(inserted.iter().copied());
            if !removed.is_empty() {
                unit.removed.extend(removed.iter().copied());
            }
        } else {
            self.commit();
            self.pending = Some(Edit::new(pos, inserted.to_vec(), removed.to_vec(), origin));
        }
        self.trim(self.limit);
    }

    /// Folds any pending burst into the undo stack.
    pub fn commit(&mut self) {
        if let Some(edit) = self.pending.take() {
            if !edit.is_empty() {
                self.undo.push(edit);
            }
        }
        self.trim(self.limit);
    }

    /// Undoes the most recent unit; returns the edit that must be applied.
    pub fn undo(&mut self) -> Option<Edit> {
        self.commit();
        let edit = self.undo.pop()?;
        self.redo.push(edit.clone());
        self.trim(self.limit);
        Some(edit.reverse())
    }

    /// Redoes the most recent undone edit.
    pub fn redo(&mut self) -> Option<Edit> {
        let edit = self.redo.pop()?;
        self.undo.push(edit.clone());
        self.trim(self.limit);
        Some(edit)
    }

    /// Discards all history (e.g. after a replace-all or remote sync).
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.pending = None;
    }

    #[must_use]
    pub fn undos(&self) -> usize {
        self.undo.len() + usize::from(self.pending.is_some())
    }

    #[must_use]
    pub fn redos(&self) -> usize {
        self.redo.len()
    }

    fn trim(&mut self, limit: usize) {
        if self.undo.len() > limit {
            let excess = self.undo.len() - limit;
            self.undo.drain(0..excess);
        }
    }
}

impl Edit {
    /// Produces the inverse edit: deleting the text this unit inserted (or
    /// re-inserting what it removed) restores the prior buffer state.
    #[must_use]
    fn reverse(&self) -> Edit {
        Edit {
            pos: self.pos,
            inserted: self.removed.clone(),
            removed: self.inserted.clone(),
            origin: self.origin,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directional_edits_undo_and_redo() {
        let mut m = UndoManager::with_limit(64);
        let chars = |s: &str| -> Vec<char> { s.chars().collect() };
        m.record(0, &chars("ab"), &[], Origin::Local);
        m.record(5, &[], &chars("xy"), Origin::Local);
        assert_eq!(m.undos(), 2);

        let unit = m.undo().unwrap();
        // Undo is a unit that re-inserts "xy" and removes nothing backward:
        // reverse of delete-at-5 is insert-at-5 ("xy").
        assert_eq!(unit.pos, 5);
        assert_eq!(unit.inserted, chars("xy"));

        let unit = m.undo().unwrap();
        assert_eq!(unit.removed, chars("ab"));
        assert_eq!(m.undos(), 0);

        assert!(m.redo().is_some());
        assert!(m.redo().is_some());
        assert!(m.redo().is_none());
        assert_eq!(m.redos(), 0);
    }

    #[test]
    fn typing_burst_collapses_into_one_unit() {
        let mut m = UndoManager::with_limit(64);
        let chars = |s: &str| -> Vec<char> { s.chars().collect() };
        for c in "hello".chars() {
            m.record(0, &[c], &[], Origin::Local);
        }
        m.commit();
        assert_eq!(m.undos(), 1);
        let unit = m.undo().unwrap();
        assert_eq!(unit.removed, chars("hello"));
    }

    #[test]
    fn history_is_bounded() {
        let mut m = UndoManager::with_limit(3);
        let e = vec!['q'];
        for i in 0..10 {
            m.record(i, &e, &[], Origin::Programmatic);
        }
        m.commit();
        assert_eq!(m.undos(), 3);
    }

    #[test]
    fn clear_drops_everything() {
        let mut m = UndoManager::with_limit(8);
        m.record(0, &['x'], &[], Origin::Local);
        m.commit();
        assert!(!m.is_clean());
        m.clear();
        assert!(m.is_clean());
        assert_eq!(m.undos(), 0);
        assert!(m.undo().is_none());
    }

    #[test]
    fn pending_edit_counts_and_commits() {
        let mut m = UndoManager::with_limit(8);
        assert!(m.is_clean());
        m.record(0, &['a'], &[], Origin::Local);
        assert_eq!(m.undos(), 1); // pending counts
        m.undo().unwrap();
        assert!(m.is_clean());
        assert_eq!(m.undos(), 0);
    }

    #[test]
    fn zero_length_edits_are_ignored() {
        let mut m = UndoManager::with_limit(8);
        m.record(0, &[], &[], Origin::Local);
        assert!(m.is_clean());
        assert!(m.undo().is_none());
    }
}
